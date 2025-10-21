//! Linux-specific waiter queue implementation using io_uring futex operations
//!
//! This implementation provides a unified event loop on Linux by submitting
//! futex operations to io_uring, allowing both I/O and synchronization to be
//! handled through the same completion queue.
//!
//! Requirements:
//! - Linux kernel 6.7+ (for IORING_OP_FUTEX_WAIT/WAKE)
//! - io-uring crate with futex support
//!
//! Fallback: If requirements not met, falls back to generic implementation

use super::generic::WaiterQueue as GenericWaiterQueue;
use std::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Waker;

#[cfg(target_os = "linux")]
use compio_driver::{OpCode, OpEntry};

#[cfg(target_os = "linux")]
use std::pin::Pin;

#[cfg(target_os = "linux")]
use libc;

/// Global cached result of futex support detection
/// Uses lock-free atomic state machine for thread-safe lazy initialization
/// 0 = not checked yet, 1 = not supported, 2 = supported
static FUTEX_SUPPORT: AtomicU8 = AtomicU8::new(0);

const FUTEX_UNKNOWN: u8 = 0;
const FUTEX_UNSUPPORTED: u8 = 1;
const FUTEX_SUPPORTED: u8 = 2;

/// Linux waiter queue - uses io_uring futex operations when available,
/// falls back to generic implementation otherwise
pub enum WaiterQueue {
    /// io_uring futex-based implementation (unified event loop)
    IoUring(IoUringWaiterQueue),
    /// Generic fallback (parking_lot-based)
    Generic(GenericWaiterQueue),
}

impl WaiterQueue {
    /// Create a new waiter queue, using io_uring futex if available
    pub fn new() -> Self {
        // Check if kernel supports io_uring futex operations
        if supports_io_uring_futex() {
            // Using io_uring futex for unified event loop
            WaiterQueue::IoUring(IoUringWaiterQueue::new())
        } else {
            // Falling back to generic (kernel < 6.7 or futex unsupported)
            WaiterQueue::Generic(GenericWaiterQueue::new())
        }
    }

    /// Get futex word for io_uring implementation (Linux only)
    ///
    /// This is used by platform-specific Future implementations.
    /// Only available when using IoUring variant.
    ///
    /// TODO: Integrate with Semaphore/Condvar futures for full io_uring usage
    #[allow(dead_code)]
    #[cfg(target_os = "linux")]
    pub(crate) fn get_futex(&self) -> Option<Arc<AtomicU32>> {
        match self {
            WaiterQueue::IoUring(q) => Some(q.get_futex()),
            WaiterQueue::Generic(_) => None,
        }
    }

    /// Add a waiter if condition is false
    pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: Fn() -> bool,
    {
        match self {
            WaiterQueue::IoUring(q) => q.add_waiter_if(condition, waker),
            WaiterQueue::Generic(q) => q.add_waiter_if(condition, waker),
        }
    }

    /// Wake one waiting task
    pub fn wake_one(&self) {
        match self {
            WaiterQueue::IoUring(q) => q.wake_one(),
            WaiterQueue::Generic(q) => q.wake_one(),
        }
    }

    /// Wake all waiting tasks
    pub fn wake_all(&self) {
        match self {
            WaiterQueue::IoUring(q) => q.wake_all(),
            WaiterQueue::Generic(q) => q.wake_all(),
        }
    }

    /// Get the number of waiting tasks
    pub fn waiter_count(&self) -> usize {
        match self {
            WaiterQueue::IoUring(q) => q.waiter_count(),
            WaiterQueue::Generic(q) => q.waiter_count(),
        }
    }
}

impl Default for WaiterQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl super::WaiterQueueTrait for WaiterQueue {
    fn new() -> Self {
        WaiterQueue::new()
    }

    fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: Fn() -> bool,
    {
        WaiterQueue::add_waiter_if(self, condition, waker)
    }

    fn wake_one(&self) {
        WaiterQueue::wake_one(self)
    }

    fn wake_all(&self) {
        WaiterQueue::wake_all(self)
    }

    fn waiter_count(&self) -> usize {
        WaiterQueue::waiter_count(self)
    }
}

/// Check if kernel supports io_uring futex operations
///
/// Uses io_uring's probe mechanism to detect support for FUTEX_WAIT and FUTEX_WAKE.
/// Result is cached globally using a lock-free atomic state machine.
/// We only probe once per process.
fn supports_io_uring_futex() -> bool {
    // Check cached result first (fast path - lock-free atomic load)
    match FUTEX_SUPPORT.load(Ordering::Acquire) {
        FUTEX_SUPPORTED => return true,
        FUTEX_UNSUPPORTED => return false,
        FUTEX_UNKNOWN => {
            // Need to probe - continue below
        }
        _ => unreachable!(),
    }

    // Probe io_uring for futex support (slow path, only once)
    let supported = probe_futex_support();

    // Cache the result atomically (lock-free)
    // Note: Multiple threads might probe simultaneously on first call,
    // but that's okay - they'll all get the same result
    let result = if supported {
        FUTEX_SUPPORTED
    } else {
        FUTEX_UNSUPPORTED
    };
    FUTEX_SUPPORT.store(result, Ordering::Release);

    supported
}

/// Probe io_uring for futex operation support
///
/// Creates a temporary io_uring instance and checks if FUTEX_WAIT/WAKE are available.
fn probe_futex_support() -> bool {
    // Try to create io_uring instance
    let ring = match io_uring::IoUring::new(2) {
        Ok(r) => r,
        Err(_) => return false,
    };

    // Create and register probe
    let mut probe = io_uring::Probe::new();

    if ring.submitter().register_probe(&mut probe).is_err() {
        return false;
    }

    // Check if FUTEX_WAIT and FUTEX_WAKE opcodes are supported
    let has_wait = probe.is_supported(io_uring::opcode::FutexWait::CODE);
    let has_wake = probe.is_supported(io_uring::opcode::FutexWake::CODE);

    has_wait && has_wake
}

/// io_uring-based waiter queue implementation
///
/// Uses futex operations submitted to io_uring for unified event loop.
/// No explicit waker queue - kernel manages waiters via futex.
///
/// Note: This is a simpler design than queue-based approaches:
/// - WaiterQueue just provides the futex word
/// - Semaphore/Condvar futures submit operations directly to compio
/// - compio's runtime handles waker management
pub struct IoUringWaiterQueue {
    /// Futex word for wait/wake operations
    /// Using AtomicU32 because futex operates on u32
    futex: Arc<AtomicU32>,

    /// Waiter count (approximate, for debugging)
    waiter_count: AtomicUsize,
}

/// Submit futex wake operation
///
/// Submits a futex wake operation to io_uring if in runtime context.
/// Falls back to direct syscall if not in runtime (e.g., during drop in sync tests).
fn submit_futex_wake(op: FutexWakeOp) {
    // Check if we're in a runtime context first
    // Runtime::with_current will panic if not in runtime
    let in_runtime = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compio::runtime::Runtime::with_current(|_| ())
    }))
    .is_ok();

    if !in_runtime {
        // Not in runtime context (e.g., sync test calling drop())
        // CRITICAL: Must issue direct syscall or waiters will hang!
        // Futex value increment alone doesn't wake - need explicit FUTEX_WAKE syscall
        #[cfg(target_os = "linux")]
        unsafe {
            let futex_ptr = Arc::as_ptr(&op.futex) as *const u32;
            libc::syscall(
                libc::SYS_futex,
                futex_ptr,
                libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                op.count as libc::c_int,
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0,
            );
        }
        return;
    }

    // Spawn task to submit wake via io_uring (fire-and-forget)
    let _handle = compio::runtime::spawn(async move {
        let _ = compio::runtime::submit(op).await;
    });
}

impl IoUringWaiterQueue {
    /// Create a new io_uring-based waiter queue
    pub fn new() -> Self {
        Self {
            futex: Arc::new(AtomicU32::new(0)),
            waiter_count: AtomicUsize::new(0),
        }
    }

    /// Get the futex word for operations to wait on
    ///
    /// This is used by platform-specific Future implementations to create
    /// futex wait operations.
    ///
    /// TODO: Integrate with Semaphore/Condvar futures for full io_uring usage
    #[allow(dead_code)]
    pub(crate) fn get_futex(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.futex)
    }

    /// Add a waiter if condition is false
    ///
    /// For io_uring implementation, this is a simplified check.
    /// The actual futex submission happens in the Future's poll() method.
    pub fn add_waiter_if<F>(&self, condition: F, _waker: Waker) -> bool
    where
        F: Fn() -> bool,
    {
        // FAST PATH: Check condition first (pure userspace)
        if condition() {
            return true;
        }

        // For io_uring implementation, we don't submit here
        // The Future's poll() method will handle submission
        // Just track that we have a waiter
        self.waiter_count.fetch_add(1, Ordering::Relaxed);

        // Post-registration recheck to reduce lost-wake window
        // (matches generic implementation pattern)
        if condition() {
            // Condition became true, remove our registration
            self.waiter_count.fetch_sub(1, Ordering::Relaxed);
            return true;
        }

        false // Pending - Future will handle futex submission
    }

    /// Wake one waiting task
    pub fn wake_one(&self) {
        // Increment futex value (this signals change to waiters)
        self.futex.fetch_add(1, Ordering::Release);

        // Decrement waiter count atomically with saturation
        // Use fetch_update to prevent underflow from concurrent decrements
        let _ = self.waiter_count.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |c| c.checked_sub(1),
        );

        // Submit futex wake operation to io_uring
        let op = FutexWakeOp::new(Arc::clone(&self.futex), 1);
        submit_futex_wake(op);

        // Note: Wake happens asynchronously through io_uring
        // The futex wait operations will complete and their futures will wake
    }

    /// Wake all waiting tasks
    pub fn wake_all(&self) {
        // Increment futex value
        self.futex.fetch_add(1, Ordering::Release);

        // Reset waiter count
        self.waiter_count.store(0, Ordering::Relaxed);

        // Submit futex wake operation to wake all waiters
        // Use u32::MAX to wake all possible waiters
        let op = FutexWakeOp::new(Arc::clone(&self.futex), u32::MAX);
        submit_futex_wake(op);
    }

    /// Get waiter count (approximate)
    pub fn waiter_count(&self) -> usize {
        self.waiter_count.load(Ordering::Relaxed)
    }
}

impl Default for IoUringWaiterQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Futex wait operation for io_uring
///
/// Waits on a futex word until it changes or is explicitly woken.
/// The waker is managed by compio's runtime when this operation is submitted.
///
/// This is an internal implementation detail, not part of the public API.
///
/// TODO: Integrate with Semaphore/Condvar futures for full io_uring usage
#[allow(dead_code)]
#[cfg(target_os = "linux")]
pub(crate) struct FutexWaitOp {
    /// Shared futex word to wait on
    futex: Arc<AtomicU32>,
    /// Expected value (wait only if futex == expected)
    expected: u32,
}

impl FutexWaitOp {
    /// Create a new futex wait operation
    #[allow(dead_code)]
    pub(crate) fn new(futex: Arc<AtomicU32>, expected: u32) -> Self {
        Self { futex, expected }
    }
}

/// Make FutexWaitOp implement compio's OpCode trait
#[cfg(target_os = "linux")]
impl OpCode for FutexWaitOp {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        use io_uring::opcode;

        // Get pointer to futex word
        let futex_ptr = Arc::as_ptr(&self.futex) as *const u32;

        // Create futex wait operation
        // Parameters: futex address, expected value, mask, futex_flags
        let entry = opcode::FutexWait::new(
            futex_ptr,
            self.expected as u64, // Expected value
            u64::MAX,             // Mask (match all bits)
            0,                    // futex_flags (futex2 flags, 0 for default)
        )
        .build();

        OpEntry::Submission(entry)
    }

    // set_result not needed - compio's Future handles waking
}

/// Futex wake operation for io_uring
///
/// Wakes waiters on a futex word.
///
/// This is an internal implementation detail, not part of the public API.
pub(crate) struct FutexWakeOp {
    /// Shared futex word
    futex: Arc<AtomicU32>,
    /// Number of waiters to wake (1 for wake_one, i32::MAX for wake_all)
    count: u32,
}

impl FutexWakeOp {
    /// Create a new futex wake operation
    pub(crate) fn new(futex: Arc<AtomicU32>, count: u32) -> Self {
        Self { futex, count }
    }
}

#[cfg(target_os = "linux")]
impl OpCode for FutexWakeOp {
    fn create_entry(self: std::pin::Pin<&mut Self>) -> OpEntry {
        use io_uring::opcode;

        // Get pointer to futex word
        let futex_ptr = Arc::as_ptr(&self.futex) as *const u32;

        // Create futex wake operation
        // Parameters: futex address, count, mask (0 = any), futex_flags (0 = none)
        let entry = opcode::FutexWake::new(
            futex_ptr,
            self.count as u64, // Number to wake
            u64::MAX,          // Mask (match all bits)
            0,                 // futex_flags
        )
        .build();

        OpEntry::Submission(entry)
    }
}
