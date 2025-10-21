//! Generic cross-platform waiter queue implementation
//!
//! **Phase 1 Implementation**: Uses parking_lot's optimized mutex with a hybrid approach:
//! - Single-waiter fast path with atomic mode switching
//! - Multi-waiter path using parking_lot::Mutex + VecDeque
//!
//! **Future Phases**: Phase 2 will add lock-free queue (crossbeam-queue) for generic,
//! io_uring for Linux, and IOCP for Windows.
//!
//! Performance characteristics:
//! - Fast path (uncontended): Userspace atomic CAS (~nanoseconds)
//! - Slow path (contended): Fast parking_lot mutex (2-5x faster than std::Mutex)
//! - No kernel involvement except waker.wake() which goes to the runtime

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::Waker;

use super::WaiterQueueTrait;

// Phase 1: parking_lot + AtomicWaker
// - AtomicWaker for single-waiter fast path (lock-free!)
// - parking_lot::Mutex for multi-waiter slow path

use atomic_waker::AtomicWaker;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use parking_lot::Mutex;

/// Modes for the waiter queue
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
enum Mode {
    /// No waiters in the queue
    Empty = 0,
    /// Exactly one waiter (uses AtomicWaker, lock-free!)
    Single = 1,
    /// Multiple waiters (uses Mutex<VecDeque>)
    Multi = 2,
}

/// Generic waiter queue implementation (Phase 1)
///
/// Uses a hybrid approach:
/// - Single waiter fast path: AtomicWaker (lock-free!)
/// - Multiple waiters slow path: parking_lot::Mutex + VecDeque
///
/// This provides optimal performance for the common case (single waiter)
/// while still handling high contention gracefully.
pub struct WaiterQueue {
    /// Current mode (empty, single, or multi)
    mode: AtomicU8,

    /// Fast path: single waiter storage (lock-free!)
    /// AtomicWaker uses pure atomic operations, no mutex needed
    single: AtomicWaker,

    /// Slow path: multiple waiters
    multi: Mutex<VecDeque<Waker>>,
}

impl WaiterQueue {
    /// Create a new waiter queue
    pub fn new() -> Self {
        Self {
            mode: AtomicU8::new(Mode::Empty.into()),
            single: AtomicWaker::new(),
            multi: Mutex::new(VecDeque::new()),
        }
    }

    /// Load the current mode
    #[inline]
    fn load_mode(&self, ordering: Ordering) -> Mode {
        // SAFETY: Mode is repr(u8) with values 0,1,2 only
        // The atomic ensures we only ever store valid Mode values
        Mode::try_from(self.mode.load(ordering))
            .expect("Invalid mode value in atomic")
    }

    /// Store a new mode
    #[inline]
    fn store_mode(&self, mode: Mode, ordering: Ordering) {
        self.mode.store(mode.into(), ordering);
    }

    /// Compare-and-exchange the mode
    #[inline]
    fn compare_exchange_mode(
        &self,
        current: Mode,
        new: Mode,
        success: Ordering,
        failure: Ordering,
    ) -> Result<Mode, Mode> {
        self.mode
            .compare_exchange(current.into(), new.into(), success, failure)
            .map(|v| Mode::try_from(v).expect("Invalid mode value in atomic"))
            .map_err(|v| Mode::try_from(v).expect("Invalid mode value in atomic"))
    }

    /// Add a waiter to the queue if condition is false (atomic check-and-add)
    ///
    /// This provides the critical race-free pattern:
    /// - Checks condition INSIDE critical section
    /// - Adds waiter only if condition is false
    /// - Re-checks after registration to prevent lost wakeups
    ///
    /// Returns:
    /// - `true` if condition was true (ready immediately)
    /// - `false` if condition was false (waiter added, pending)
    pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: Fn() -> bool,
    {
        // Try single-waiter fast path first
        let mode = self.load_mode(Ordering::Acquire);

        if mode == Mode::Empty {
            // Try to transition EMPTY → SINGLE atomically
            if self
                .compare_exchange_mode(
                    Mode::Empty,
                    Mode::Single,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                // Successfully claimed single slot - use lock-free AtomicWaker!
                
                // Check before registration
                if condition() {
                    self.store_mode(Mode::Empty, Ordering::Release);
                    return true;
                }

                // Register with AtomicWaker (lock-free atomic operation!)
                self.single.register(&waker);

                // Re-check after registration to prevent lost wake
                if condition() {
                    // Take the waker back and reset mode
                    self.single.take();
                    self.store_mode(Mode::Empty, Ordering::Release);
                    return true;
                }
                
                // Successfully registered, pending
                return false;
            }
        }

        // Multiple waiters or contention → use multi queue (and migrate single)
        // Check condition before taking locks
        if condition() {
            return true;
        }
        
        let mut waiters = self.multi.lock();
        
        // Migrate single-slot waiter if present (atomically take it)
        if let Some(prev) = self.single.take() {
            waiters.push_back(prev);
        }
        
        // Register this waiter
        waiters.push_back(waker);
        
        // Re-check after registration to prevent lost wake
        if condition() {
            // Remove our own registration
            let _ = waiters.pop_back();
            // If nothing remains, update mode accordingly
            if waiters.is_empty() {
                self.store_mode(Mode::Empty, Ordering::Release);
            } else {
                self.store_mode(Mode::Multi, Ordering::Release);
            }
            return true;
        }
        
        self.store_mode(Mode::Multi, Ordering::Release);
        false
    }

    /// Wake one waiting task
    pub fn wake_one(&self) {
        let mode = self.load_mode(Ordering::Acquire);

        match mode {
            Mode::Empty => {
                // No waiters, nothing to do
            }
            Mode::Single => {
                // Lock-free atomic wake using AtomicWaker!
                if let Some(w) = self.single.take() {
                    // Check if multi has waiters to decide next mode
                    let has_multi = { !self.multi.lock().is_empty() };
                    self.store_mode(
                        if has_multi { Mode::Multi } else { Mode::Empty },
                        Ordering::Release,
                    );
                    w.wake();
                } else {
                    // Nothing in single (registration race) → try multi, then fix mode
                    if !self.wake_one_from_multi() {
                        // Both empty, check and update mode appropriately
                        let has_multi = { !self.multi.lock().is_empty() };
                        self.store_mode(
                            if has_multi { Mode::Multi } else { Mode::Empty },
                            Ordering::Release,
                        );
                    }
                }
            }
            Mode::Multi => {
                // Prefer multi; if empty, try single and update mode accordingly
                if !self.wake_one_from_multi() {
                    // Try single waiter (lock-free!)
                    if let Some(w) = self.single.take() {
                        // Check if multi still has waiters for next mode
                        let has_multi = { !self.multi.lock().is_empty() };
                        self.store_mode(
                            if has_multi { Mode::Multi } else { Mode::Empty },
                            Ordering::Release,
                        );
                        w.wake();
                    } else {
                        // Both empty, reset mode
                        self.store_mode(Mode::Empty, Ordering::Release);
                    }
                }
            }
        }
    }

    /// Wake one waiter from multi queue (internal helper)
    /// Returns true if a waiter was woken, false otherwise
    fn wake_one_from_multi(&self) -> bool {
        let waker = {
            let mut waiters = self.multi.lock();
            // If queue is now empty, defer mode update to caller
            // (caller may still need to check single slot)
            waiters.pop_front()
        };

        // Wake outside lock
        if let Some(waker) = waker {
            waker.wake();
            return true;
        }
        false
    }

    /// Wake all waiting tasks
    pub fn wake_all(&self) {
        // Drain both storages
        // Single: lock-free atomic take
        let single_waker = self.single.take();
        
        // Multi: lock and drain
        let multi_wakers = {
            let mut waiters = self.multi.lock();
            std::mem::take(&mut *waiters)
        };
        
        // Reset mode after draining
        self.store_mode(Mode::Empty, Ordering::Release);

        // Wake all outside lock
        if let Some(waker) = single_waker {
            waker.wake();
        }

        for waker in multi_wakers {
            waker.wake();
        }
    }

    /// Get the number of waiting tasks (for debugging/stats)
    ///
    /// Note: This provides a best-effort count that may be slightly
    /// inaccurate during mode transitions. AtomicWaker doesn't expose
    /// a way to check occupancy without taking the waker, so we use
    /// the mode as a hint.
    pub fn waiter_count(&self) -> usize {
        let mode = self.load_mode(Ordering::Acquire);
        let multi_count = self.multi.lock().len();
        
        match mode {
            Mode::Empty => multi_count, // Should be 0, but check multi just in case
            Mode::Single => {
                // Single waiter might exist, plus any in multi queue
                // (during migration from single to multi, both might have waiters)
                1 + multi_count
            }
            Mode::Multi => {
                // Multi queue has waiters, single might have one during migration
                // Be conservative and assume single might have one
                multi_count.saturating_add(1)
            }
        }
    }
}

impl Default for WaiterQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl WaiterQueueTrait for WaiterQueue {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::task::Wake;

    struct DummyWaker;
    impl Wake for DummyWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn dummy_waker() -> Waker {
        Arc::new(DummyWaker).into()
    }

    // CountingWaker to verify actual wakes happen
    struct CountingWaker(AtomicUsize);
    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn counting_waker(counter: &Arc<CountingWaker>) -> Waker {
        counter.clone().into()
    }

    #[test]
    fn test_empty_queue() {
        let queue = WaiterQueue::new();
        assert_eq!(queue.waiter_count(), 0);
        assert_eq!(queue.load_mode(Ordering::Relaxed), Mode::Empty);
    }

    #[test]
    fn test_single_waiter() {
        let queue = WaiterQueue::new();

        // Add single waiter
        let result = queue.add_waiter_if(|| false, dummy_waker());
        assert!(!result);
        assert_eq!(queue.waiter_count(), 1);

        // Wake it
        queue.wake_one();
        assert_eq!(queue.waiter_count(), 0);
    }

    #[test]
    fn test_multiple_waiters() {
        let queue = WaiterQueue::new();

        // Add multiple waiters
        queue.add_waiter_if(|| false, dummy_waker());
        queue.add_waiter_if(|| false, dummy_waker());
        queue.add_waiter_if(|| false, dummy_waker());

        let count = queue.waiter_count();
        assert!(count >= 1, "Should have at least 1 waiter, got {}", count);

        // Wake one
        queue.wake_one();

        // Should have fewer waiters now
        let count_after_wake_one = queue.waiter_count();
        assert!(
            count_after_wake_one < count,
            "Should have fewer waiters after wake_one"
        );

        // Wake all
        queue.wake_all();
        assert_eq!(
            queue.waiter_count(),
            0,
            "Should have no waiters after wake_all"
        );
    }

    #[test]
    fn test_condition_check() {
        let queue = WaiterQueue::new();

        // Condition true - should not add
        let result = queue.add_waiter_if(|| true, dummy_waker());
        assert!(result);
        assert_eq!(queue.waiter_count(), 0);

        // Condition false - should add
        let result = queue.add_waiter_if(|| false, dummy_waker());
        assert!(!result);
        assert_eq!(queue.waiter_count(), 1);
    }

    #[test]
    fn test_wake_all_empty() {
        let queue = WaiterQueue::new();
        // Should not panic
        queue.wake_all();
        assert_eq!(queue.waiter_count(), 0);
    }

    #[test]
    fn test_wake_one_calls_waker() {
        let queue = WaiterQueue::new();
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        
        // Add waiter
        queue.add_waiter_if(|| false, counting_waker(&counter));
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        
        // Wake should call the waker
        queue.wake_one();
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_wake_all_calls_all_wakers() {
        let queue = WaiterQueue::new();
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        
        // Add 5 waiters
        for _ in 0..5 {
            queue.add_waiter_if(|| false, counting_waker(&counter));
        }
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        
        // Wake all should call all 5 wakers
        queue.wake_all();
        assert_eq!(counter.0.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn test_wake_one_multiple_waiters() {
        let queue = WaiterQueue::new();
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        
        // Add 3 waiters
        for _ in 0..3 {
            queue.add_waiter_if(|| false, counting_waker(&counter));
        }
        
        // Wake one at a time
        queue.wake_one();
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        
        queue.wake_one();
        assert_eq!(counter.0.load(Ordering::Relaxed), 2);
        
        queue.wake_one();
        assert_eq!(counter.0.load(Ordering::Relaxed), 3);
    }
}
