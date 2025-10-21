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

// TODO: Decide between parking_lot or crossbeam after benchmarking
// For now, using parking_lot as it's simpler and proven

use parking_lot::Mutex;

/// Modes for the waiter queue
const MODE_EMPTY: u8 = 0;
const MODE_SINGLE: u8 = 1;
const MODE_MULTI: u8 = 2;

/// Generic waiter queue implementation (Phase 1)
///
/// Uses a hybrid approach:
/// - Single waiter fast path (atomic mode + parking_lot mutex for storage)
/// - Multiple waiters slow path (parking_lot mutex + VecDeque)
pub struct WaiterQueue {
    /// Current mode (empty, single, or multi)
    mode: AtomicU8,

    /// Fast path: single waiter storage
    /// Using Option to avoid allocation when no waiter
    single: Mutex<Option<Waker>>,

    /// Slow path: multiple waiters
    multi: Mutex<VecDeque<Waker>>,
}

impl WaiterQueue {
    /// Create a new waiter queue
    pub fn new() -> Self {
        Self {
            mode: AtomicU8::new(MODE_EMPTY),
            single: Mutex::new(None),
            multi: Mutex::new(VecDeque::new()),
        }
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
        let mode = self.mode.load(Ordering::Acquire);

        if mode == MODE_EMPTY {
            // Try to transition EMPTY → SINGLE atomically
            if self
                .mode
                .compare_exchange(MODE_EMPTY, MODE_SINGLE, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Successfully claimed single slot
                {
                    let mut single = self.single.lock();

                    // Check before registration
                    if condition() {
                        self.mode.store(MODE_EMPTY, Ordering::Release);
                        return true;
                    }

                    // Register
                    *single = Some(waker);

                    // Re-check after registration to prevent lost wake
                    if condition() {
                        // Remove and reset mode
                        let _ = single.take();
                        self.mode.store(MODE_EMPTY, Ordering::Release);
                        return true;
                    }
                }
                return false; // pending
            }
        }

        // Multiple waiters or contention → use multi queue (and migrate single)
        // Lock order: single → multi (matches wake_all) to avoid deadlocks.
        let mut single = self.single.lock();
        if condition() {
            return true;
        }
        let mut waiters = self.multi.lock();
        // Migrate single-slot waiter if present
        if let Some(prev) = single.take() {
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
                self.mode.store(MODE_EMPTY, Ordering::Release);
            } else {
                self.mode.store(MODE_MULTI, Ordering::Release);
            }
            return true;
        }
        self.mode.store(MODE_MULTI, Ordering::Release);
        false
    }

    /// Wake one waiting task
    pub fn wake_one(&self) {
        let mode = self.mode.load(Ordering::Acquire);

        match mode {
            MODE_EMPTY => {
                // No waiters, nothing to do
            }
            MODE_SINGLE => {
                // Try to wake single waiter
                if self
                    .mode
                    .compare_exchange(MODE_SINGLE, MODE_EMPTY, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    // Successfully transitioned to EMPTY
                    let waker = {
                        let mut single = self.single.lock();
                        single.take()
                    };

                    // Wake outside lock
                    if let Some(waker) = waker {
                        waker.wake();
                    }
                } else {
                    // Raced with another waiter being added
                    // Fall through to multi queue
                    self.wake_one_from_multi();
                }
            }
            MODE_MULTI => {
                // Prefer multi; if empty, also try single
                if !self.wake_one_from_multi() {
                    let w = { self.single.lock().take() };
                    if let Some(w) = w {
                        w.wake();
                    } else {
                        // Both empty, reset mode
                        self.mode.store(MODE_EMPTY, Ordering::Release);
                    }
                }
            }
            _ => unreachable!("Invalid mode"),
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
        // Drain both storages (lock order: single → multi)
        let single_waker = { self.single.lock().take() };
        let multi_wakers = {
            let mut waiters = self.multi.lock();
            std::mem::take(&mut *waiters)
        };
        // Reset mode after draining
        self.mode.store(MODE_EMPTY, Ordering::Release);

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
    /// Note: This locks both storages to get a consistent count,
    /// preventing races with concurrent mode transitions.
    pub fn waiter_count(&self) -> usize {
        // Lock order: single → multi (consistent with elsewhere)
        // This prevents TOCTOU races where mode changes between reads
        let single = self.single.lock();
        let multi = self.multi.lock();
        
        let single_count = if single.is_some() { 1 } else { 0 };
        let multi_count = multi.len();
        
        single_count + multi_count
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
        assert_eq!(queue.mode.load(Ordering::Relaxed), MODE_EMPTY);
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
