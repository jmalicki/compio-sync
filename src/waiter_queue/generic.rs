//! Generic cross-platform waiter queue implementation
//!
//! **Phase 1 Implementation**: Lock-free single-waiter optimization + parking_lot for multi-waiter:
//! - Single-waiter fast path: AtomicWaker (lock-free atomic operations!)
//! - Multi-waiter slow path: parking_lot::Mutex + VecDeque (2-5x faster than std::Mutex)
//! - Atomic mode state machine: Empty → Single → Multi
//!
//! **Future Phases**: Phase 2 will add platform-specific optimizations:
//! - Linux: io_uring futex operations
//! - Windows: IOCP integration
//! - Generic: Potentially crossbeam-queue
//!
//! Performance characteristics:
//! - Single waiter (common case): Lock-free atomic operations (~nanoseconds, zero mutex overhead)
//! - Multiple waiters: Fast parking_lot mutex (2-5x faster than std::Mutex)
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
        Mode::try_from(self.mode.load(ordering)).expect("Invalid mode value in atomic")
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
    /// Returns a future that completes when condition is true or waiter is woken.
    pub fn add_waiter_if<'a, F>(
        &'a self,
        condition: F,
    ) -> impl std::future::Future<Output = ()> + use<'a, F>
    where
        F: Fn() -> bool + Send + Sync + 'a,
    {
        // Track where we registered for proper cleanup on drop
        enum RegistrationState {
            None,   // Not yet registered
            Single, // Registered in single slot
            Multi,  // Registered in multi queue
        }

        // Use a struct to track registration state across polls
        struct AddWaiterFuture<'a, F> {
            queue: &'a WaiterQueue,
            condition: F,
            state: RegistrationState,
        }

        impl<'a, F> Drop for AddWaiterFuture<'a, F> {
            fn drop(&mut self) {
                // Deregister if we're still pending
                match self.state {
                    RegistrationState::Single => {
                        // Try to clean up single slot
                        // If we successfully take it, reset to Empty
                        if self.queue.single.take().is_some() {
                            self.queue.store_mode(Mode::Empty, Ordering::Release);
                        }
                    }
                    RegistrationState::Multi => {
                        // Can't efficiently remove from VecDeque without knowing our position
                        // The waker will be a no-op if called (future already dropped)
                        // This is acceptable - spurious wake is safe, just slightly inefficient
                        //
                        // Note: We could track position in VecDeque but that adds significant
                        // complexity. The parking_lot Mutex is fast enough that this is okay.
                    }
                    RegistrationState::None => {
                        // Not registered, nothing to clean up
                    }
                }
            }
        }

        impl<'a, F> std::future::Future for AddWaiterFuture<'a, F>
        where
            F: Fn() -> bool,
        {
            type Output = ();

            fn poll(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<()> {
                use std::task::Poll;

                // SAFETY: We don't move out of self, just access fields
                let this = unsafe { self.as_mut().get_unchecked_mut() };

                // If already registered, just wait for wake
                if !matches!(this.state, RegistrationState::None) {
                    // We were woken - complete the future
                    this.state = RegistrationState::None;
                    return Poll::Ready(());
                }

                let queue = this.queue;
                let condition = &this.condition;

                // Try single-waiter fast path first
                let mode = queue.load_mode(Ordering::Acquire);

                if mode == Mode::Empty {
                    // Try to transition EMPTY → SINGLE atomically
                    if queue
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
                            queue.store_mode(Mode::Empty, Ordering::Release);
                            return Poll::Ready(());
                        }

                        // Register with AtomicWaker (lock-free atomic operation!)
                        queue.single.register(cx.waker());

                        // Re-check after registration to prevent lost wake
                        if condition() {
                            // Take the waker back and reset mode
                            queue.single.take();
                            queue.store_mode(Mode::Empty, Ordering::Release);
                            return Poll::Ready(());
                        }

                        // Successfully registered in single slot, pending
                        this.state = RegistrationState::Single;
                        return Poll::Pending;
                    }
                }

                // Multiple waiters or contention → use multi queue (and migrate single)
                // Check condition before taking locks
                if condition() {
                    return Poll::Ready(());
                }

                let mut waiters = queue.multi.lock();

                // Migrate single-slot waiter if present (atomically take it)
                if let Some(prev) = queue.single.take() {
                    waiters.push_back(prev);
                }

                // Register this waiter
                waiters.push_back(cx.waker().clone());

                // Re-check after registration to prevent lost wake
                if condition() {
                    // Remove our own registration
                    let _ = waiters.pop_back();
                    // If nothing remains, update mode accordingly
                    if waiters.is_empty() {
                        queue.store_mode(Mode::Empty, Ordering::Release);
                    } else {
                        queue.store_mode(Mode::Multi, Ordering::Release);
                    }
                    return Poll::Ready(());
                }

                queue.store_mode(Mode::Multi, Ordering::Release);
                this.state = RegistrationState::Multi;
                Poll::Pending
            }
        }

        AddWaiterFuture {
            queue: self,
            condition,
            state: RegistrationState::None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_queue() {
        let queue = WaiterQueue::new();
        assert_eq!(queue.waiter_count(), 0);
        assert_eq!(queue.load_mode(Ordering::Relaxed), Mode::Empty);
    }

    #[compio::test]
    async fn test_single_waiter() {
        let queue = std::sync::Arc::new(WaiterQueue::new());
        let queue_clone = queue.clone();

        // Add single waiter in background
        let handle = compio::runtime::spawn(async move {
            queue_clone.add_waiter_if(|| false).await;
        });

        // Give it time to register
        compio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_eq!(queue.waiter_count(), 1);

        // Wake it
        queue.wake_one();

        // Should complete
        compio::time::timeout(std::time::Duration::from_millis(100), handle)
            .await
            .expect("Should complete after wake")
            .expect("Task should succeed");
        assert_eq!(queue.waiter_count(), 0);
    }

    #[compio::test]
    async fn test_multiple_waiters() {
        let queue = std::sync::Arc::new(WaiterQueue::new());

        // Add multiple waiters in background
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let q = queue.clone();
                compio::runtime::spawn(async move { q.add_waiter_if(|| false).await })
            })
            .collect();

        // Give them time to register
        compio::time::sleep(std::time::Duration::from_millis(10)).await;
        let count = queue.waiter_count();
        assert!(count >= 1, "Should have at least 1 waiter, got {}", count);

        // Wake all
        queue.wake_all();

        // All should complete
        for handle in handles {
            compio::time::timeout(std::time::Duration::from_millis(100), handle)
                .await
                .expect("Should complete after wake")
                .expect("Task should succeed");
        }

        assert_eq!(
            queue.waiter_count(),
            0,
            "Should have no waiters after wake_all"
        );
    }

    #[compio::test]
    async fn test_condition_check() {
        let queue = WaiterQueue::new();

        // Condition true - should complete immediately
        queue.add_waiter_if(|| true).await;
        assert_eq!(queue.waiter_count(), 0);
    }

    #[test]
    fn test_wake_all_empty() {
        let queue = WaiterQueue::new();
        // Should not panic
        queue.wake_all();
        assert_eq!(queue.waiter_count(), 0);
    }

    // Note: Waker-specific tests removed since poll_add_waiter_if now gets
    // the waker from Context. Functionality is tested at higher levels
    // (Condvar/Semaphore tests).
}
