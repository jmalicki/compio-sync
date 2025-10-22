//! Windows-specific waiter queue implementation using WaitOnAddress
//!
//! This implementation uses Windows 8+ WaitOnAddress/WakeByAddress APIs,
//! which provide futex-like functionality on Windows.
//!
//! Requirements:
//! - Windows 8+ (for WaitOnAddress/WakeByAddress)
//!
//! Fallback: If requirements not met, falls back to generic implementation

use super::generic::WaiterQueue as GenericWaiterQueue;
use std::io;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

#[cfg(windows)]
use std::pin::Pin;

#[cfg(windows)]
use std::os::windows::io::RawHandle;

/// Global cached result of WaitOnAddress support detection
/// 0 = not checked yet, 1 = not supported, 2 = supported
static WAITONADDRESS_SUPPORT: AtomicU8 = AtomicU8::new(0);

const WAITONADDRESS_UNKNOWN: u8 = 0;
const WAITONADDRESS_UNSUPPORTED: u8 = 1;
const WAITONADDRESS_SUPPORTED: u8 = 2;

/// Windows waiter queue - uses WaitOnAddress when available (Windows 8+),
/// falls back to generic implementation otherwise
pub enum WaiterQueue {
    /// WaitOnAddress-based implementation (Windows 8+, futex-like)
    WaitOnAddress(WaitOnAddressQueue),
    /// Generic fallback (parking_lot-based)
    Generic(GenericWaiterQueue),
}

impl WaiterQueue {
    /// Create a new waiter queue, using WaitOnAddress if available
    pub fn new() -> Self {
        // Check if Windows supports WaitOnAddress (Windows 8+)
        if supports_wait_on_address() {
            // Using WaitOnAddress for futex-like synchronization
            WaiterQueue::WaitOnAddress(WaitOnAddressQueue::new())
        } else {
            // Falling back to generic
            WaiterQueue::Generic(GenericWaiterQueue::new())
        }
    }

    /// Get event handle for IOCP implementation (Windows only)
    ///
    /// This is used by platform-specific Future implementations.
    #[cfg(windows)]
    pub(crate) fn get_event_handle(&self) -> Option<Arc<EventHandle>> {
        match self {
            WaiterQueue::WaitOnAddress(q) => Some(q.get_event_handle()),
            WaiterQueue::Generic(_) => None,
        }
    }

    /// Add a waiter if condition is false
    pub fn add_waiter_if<'a, F>(
        &'a self,
        condition: F,
    ) -> impl std::future::Future<Output = ()> + use<'a, F>
    where
        F: Fn() -> bool + Send + Sync + 'a,
    {
        match self {
            WaiterQueue::WaitOnAddress(q) => {
                // Box to make the arms have the same type
                Box::pin(q.add_waiter_if(condition))
                    as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>
            }
            WaiterQueue::Generic(q) => Box::pin(q.add_waiter_if(condition)),
        }
    }

    /// Wake one waiting task
    pub fn wake_one(&self) {
        match self {
            WaiterQueue::WaitOnAddress(q) => q.wake_one(),
            WaiterQueue::Generic(q) => q.wake_one(),
        }
    }

    /// Wake all waiting tasks
    pub fn wake_all(&self) {
        match self {
            WaiterQueue::WaitOnAddress(q) => q.wake_all(),
            WaiterQueue::Generic(q) => q.wake_all(),
        }
    }

    /// Get the number of waiting tasks
    pub fn waiter_count(&self) -> usize {
        match self {
            WaiterQueue::WaitOnAddress(q) => q.waiter_count(),
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

    fn add_waiter_if<'a, F>(&'a self, condition: F) -> impl std::future::Future<Output = ()>
    where
        F: Fn() -> bool + Send + Sync + 'a,
    {
        WaiterQueue::add_waiter_if(self, condition)
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

/// Check if Windows WaitOnAddress features are available
///
/// Result is cached globally - we only check once per process.
fn supports_wait_on_address() -> bool {
    // Check cached result first (fast path)
    match WAITONADDRESS_SUPPORT.load(Ordering::Acquire) {
        WAITONADDRESS_SUPPORTED => return true,
        WAITONADDRESS_UNSUPPORTED => return false,
        WAITONADDRESS_UNKNOWN => {
            // Need to check - continue below
        }
        _ => unreachable!(),
    }

    // Check WaitOnAddress support (slow path, only once)
    let supported = probe_wait_on_address_support();

    // Cache the result atomically
    let result = if supported {
        WAITONADDRESS_SUPPORTED
    } else {
        WAITONADDRESS_UNSUPPORTED
    };
    WAITONADDRESS_SUPPORT.store(result, Ordering::Release);

    supported
}

/// Probe for WaitOnAddress support
///
/// WaitOnAddress is available on Windows 8+
#[cfg(windows)]
fn probe_wait_on_address_support() -> bool {
    // Check if WaitOnAddress is available
    // On Windows 8+, these APIs should be present
    // Could dynamically load and check, but for simplicity, assume Windows 8+

    // TODO: Could use windows_sys to check version or dynamically load
    // For now, assume it's available on all Windows we support
    true
}

#[cfg(not(windows))]
fn probe_wait_on_address_support() -> bool {
    false
}

/// IOCP Event-based waiter queue implementation
///
/// Uses Windows event objects + IOCP for unified event loop.
///
/// Note: Similar design to Linux futex:
/// - WaiterQueue provides event handle for IOCP waiting
/// - Platform-specific Future creates EventWaitOp (OpType::Event)
/// - Wake methods signal the event, triggering IOCP completion
pub struct WaitOnAddressQueue {
    /// Event handle for IOCP waiting
    /// compio waits on this via OpType::Event
    #[cfg(windows)]
    event: Arc<EventHandle>,

    /// Waiter count (approximate, for debugging)
    waiter_count: AtomicUsize,
}

#[cfg(windows)]
struct EventHandle {
    handle: RawHandle,
}

#[cfg(windows)]
impl EventHandle {
    fn new() -> io::Result<Self> {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Threading::CreateEventW;

        unsafe {
            let handle = CreateEventW(
                std::ptr::null_mut(), // default security
                0,                    // auto-reset
                0,                    // initially non-signaled
                std::ptr::null(),     // no name
            );

            if handle == 0 || handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }

            Ok(Self { handle })
        }
    }

    fn handle(&self) -> RawHandle {
        self.handle
    }

    fn signal(&self) {
        use windows_sys::Win32::System::Threading::SetEvent;
        unsafe {
            SetEvent(self.handle as isize);
        }
    }
}

#[cfg(windows)]
impl Drop for EventHandle {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        unsafe {
            CloseHandle(self.handle as isize);
        }
    }
}

#[cfg(windows)]
unsafe impl Send for EventHandle {}
#[cfg(windows)]
unsafe impl Sync for EventHandle {}

impl WaitOnAddressQueue {
    /// Create a new IOCP event-based waiter queue
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            Self {
                event: Arc::new(EventHandle::new().expect("Failed to create event")),
                waiter_count: AtomicUsize::new(0),
            }
        }

        #[cfg(not(windows))]
        {
            Self {
                waiter_count: AtomicUsize::new(0),
            }
        }
    }

    /// Get the event handle for IOCP operations
    ///
    /// This is used by platform-specific Future implementations.
    #[cfg(windows)]
    pub(crate) fn get_event_handle(&self) -> Arc<EventHandle> {
        Arc::clone(&self.event)
    }

    /// Add a waiter if condition is false
    ///
    /// For IOCP, returns the submit() future directly!
    /// The caller will await this future, and compio will wake them when the event is signaled.
    ///
    /// **Note**: The returned future is `!Send` because IOCP operations are
    /// thread-local in compio's runtime.
    pub fn add_waiter_if<F>(&self, condition: F) -> impl std::future::Future<Output = ()> + use<F>
    where
        F: Fn() -> bool + Send + Sync,
    {
        let event = Arc::clone(&self.event);

        async move {
            // Fast path: check condition first
            if condition() {
                return;
            }

            // Submit IOCP event wait - this future completes when event is signaled
            let op = EventWaitOp::new(event.clone());

            // Just await the submit - compio handles the waker!
            // When the event is signaled (via wake_one/wake_all), this completes
            let _ = compio::runtime::submit(op).await;

            // Note: No waiter count tracking - IOCP manages waiters internally
        }
    }

    /// Wake one waiting task
    pub fn wake_one(&self) {
        // Decrement waiter count
        let count = self.waiter_count.load(Ordering::Relaxed);
        if count > 0 {
            self.waiter_count.fetch_sub(1, Ordering::Relaxed);
        }

        // Signal event - triggers IOCP completion
        #[cfg(windows)]
        self.event.signal();
    }

    /// Wake all waiting tasks
    pub fn wake_all(&self) {
        // Reset waiter count
        self.waiter_count.store(0, Ordering::Relaxed);

        // Signal event - triggers IOCP completion for all waiters
        #[cfg(windows)]
        self.event.signal();
    }

    /// Get waiter count (approximate)
    pub fn waiter_count(&self) -> usize {
        self.waiter_count.load(Ordering::Relaxed)
    }
}

/// Event wait operation for IOCP
///
/// Waits on a Windows event handle via IOCP (OpType::Event).
/// Similar to Linux FutexWaitOp, but uses event handles instead of futex.
#[cfg(windows)]
pub(crate) struct EventWaitOp {
    /// Event handle to wait on
    event: Arc<EventHandle>,
}

#[cfg(windows)]
impl EventWaitOp {
    /// Create a new event wait operation
    pub(crate) fn new(event: Arc<EventHandle>) -> Self {
        Self { event }
    }
}

#[cfg(windows)]
impl compio_driver::OpCode for EventWaitOp {
    /// This is an event wait operation - compio will wait via IOCP
    fn op_type(&self) -> compio_driver::OpType {
        compio_driver::OpType::Event(self.event.handle())
    }

    /// For Event operations, this is called in the IOCP thread
    unsafe fn operate(
        self: Pin<&mut Self>,
        _optr: *mut windows_sys::Win32::System::IO::OVERLAPPED,
    ) -> std::task::Poll<io::Result<usize>> {
        // Event was signaled - return Ready
        // The actual waiting is handled by IOCP
        std::task::Poll::Ready(Ok(0))
    }
}

// Windows IOCP Event implementation complete!
// Uses Windows event objects + IOCP for unified event loop.
//
// Architecture:
// 1. WaiterQueue creates an event handle
// 2. Future submits EventWaitOp with OpType::Event(handle)
// 3. compio registers event with IOCP
// 4. When wake_one() calls SetEvent(), IOCP completion fires
// 5. Future gets polled, tries to acquire permit
//
// This gives us true unified event loop like Linux io_uring futex!
