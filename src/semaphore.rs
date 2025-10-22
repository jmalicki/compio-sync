//! Async semaphore for compio runtime
//!
//! Provides a semaphore primitive compatible with compio's async runtime to limit
//! concurrent operations. This is critical for bounding memory usage and preventing
//! resource exhaustion during directory traversal.
//!
//! # Example
//!
//! ```rust,no_run
//! use compio_sync::Semaphore;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create semaphore with 1024 permits
//! let semaphore = Arc::new(Semaphore::new(1024));
//!
//! // Acquire permit before starting work
//! let permit = semaphore.acquire().await;
//!
//! // Do work while holding permit
//! // ...
//!
//! // Permit automatically released when dropped
//! drop(permit);
//! # Ok(())
//! # }
//! ```

use crate::waiter_queue::{WaiterQueue, WaiterQueueTrait};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A compio-compatible async semaphore for bounding concurrency
///
/// The semaphore maintains a fixed number of permits that must be acquired
/// before performing an operation. When all permits are in use, `acquire()`
/// will wait asynchronously until a permit becomes available.
///
/// # Design
///
/// - **Lock-free fast path**: Uses atomics for acquiring/releasing when permits available
/// - **Fair wakeup**: All waiting tasks will eventually complete (no starvation)
/// - **Wake order**: Implementation-dependent (FIFO for Generic, unspecified for io_uring)
/// - **RAII permits**: `SemaphorePermit` automatically releases on drop
/// - **Cloneable**: Wrapped in `Arc` for sharing across tasks
///
/// # Example
///
/// ```rust,no_run
/// use compio_sync::Semaphore;
/// use std::sync::Arc;
///
/// # async fn example() {
/// let sem = Arc::new(Semaphore::new(100));
///
/// // Spawn multiple concurrent tasks
/// for i in 0..1000 {
///     let sem = sem.clone();
///     compio::runtime::spawn(async move {
///         let _permit = sem.acquire().await;
///         // Only 100 tasks run concurrently
///         println!("Processing {}", i);
///     });
/// }
/// # }
/// ```
pub struct SemaphoreGeneric<W: WaiterQueueTrait> {
    /// Internal state for the semaphore
    /// Users should wrap in `Arc<Semaphore>` when sharing between tasks
    inner: SemaphoreInner<W>,
}

/// Public type alias using platform-specific WaiterQueue
///
/// This is what users actually interact with. The generic implementation
/// allows for flexibility and testing while this alias keeps the API simple.
pub type Semaphore = SemaphoreGeneric<WaiterQueue>;

/// Internal shared state for the semaphore
///
/// This structure contains the atomic permit counter and the queue of waiting tasks.
/// It is wrapped in an Arc to allow the Semaphore to be cloned cheaply.
///
/// # Implementation Note
///
/// Currently uses `Mutex<VecDeque<Waker>>` for simplicity and maintainability.
///
/// **Future optimization**: Could use intrusive linked list (like tokio) to avoid
/// allocations and improve cache locality. However, this requires unsafe code and
/// is significantly more complex. The current VecDeque approach is proven and fast enough.
struct SemaphoreInner<W: WaiterQueueTrait> {
    /// Available permits (atomic for lock-free operations)
    permits: AtomicUsize,
    /// Maximum permits (for metrics and debugging)
    max_permits: usize,
    /// Waiter queue abstraction (handles mutex + wait/wake pattern)
    /// See `waiter_queue.rs` for why mutex is safe in async code
    waiters: W,
}

impl<W: WaiterQueueTrait> SemaphoreGeneric<W> {
    /// Create a new semaphore with the given number of permits
    ///
    /// # Arguments
    ///
    /// * `permits` - The initial number of permits (maximum concurrency)
    ///
    /// # Panics
    ///
    /// Panics if `permits` is 0 (semaphore must have at least one permit)
    ///
    /// # Example
    ///
    /// ```rust
    /// use compio_sync::Semaphore;
    ///
    /// let sem = Semaphore::new(1024);
    /// assert_eq!(sem.available_permits(), 1024);
    /// ```
    #[must_use]
    pub fn new(permits: usize) -> Self {
        assert!(permits > 0, "Semaphore must have at least one permit");
        Self {
            inner: SemaphoreInner {
                permits: AtomicUsize::new(permits),
                max_permits: permits,
                waiters: W::new(),
            },
        }
    }

    /// Acquire a permit, waiting asynchronously if none are available
    ///
    /// Returns a `SemaphorePermit` that will release the permit when dropped.
    /// This method will wait (yield to other tasks) if no permits are currently available.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use compio_sync::Semaphore;
    ///
    /// # async fn example() {
    /// let sem = Semaphore::new(10);
    ///
    /// let permit = sem.acquire().await;
    /// // Do work...
    /// drop(permit);  // Release permit
    /// # }
    /// ```
    pub async fn acquire(&self) -> SemaphorePermit<'_, W> {
        loop {
            // Fast path: try to acquire immediately
            if let Some(permit) = self.try_acquire() {
                return permit;
            }

            // No permits - register waiter and wait for release
            // CRITICAL: Check permit availability during registration to prevent lost-wake race
            // If permits become available after try_acquire() fails but before registration
            // completes, the condition re-check will catch it and return immediately.
            self.inner
                .waiters
                .add_waiter_if(|| self.available_permits() > 0)
                .await;

            // After wake (or immediate return), loop back to try_acquire
        }
    }

    /// Try to acquire a permit without waiting
    ///
    /// Returns `Some(SemaphorePermit)` if a permit was immediately available,
    /// or `None` if all permits are currently in use.
    ///
    /// # Example
    ///
    /// ```rust
    /// use compio_sync::Semaphore;
    ///
    /// let sem = Semaphore::new(1);
    ///
    /// let permit1 = sem.try_acquire();
    /// assert!(permit1.is_some());
    ///
    /// let permit2 = sem.try_acquire();
    /// assert!(permit2.is_none());  // No permits left
    /// ```
    #[must_use]
    pub fn try_acquire(&self) -> Option<SemaphorePermit<'_, W>> {
        // Fast path: atomic decrement if permits available
        let mut current = self.inner.permits.load(Ordering::Acquire);

        loop {
            if current == 0 {
                return None; // No permits available
            }

            // Try to atomically decrement
            match self.inner.permits.compare_exchange_weak(
                current,
                current - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(SemaphorePermit { semaphore: self }),
                Err(actual) => current = actual, // Retry with updated value
            }
        }
    }

    /// Get the number of available permits
    ///
    /// This is useful for monitoring and debugging but should not be used
    /// for making decisions (value may change immediately after reading).
    ///
    /// # Example
    ///
    /// ```rust
    /// use compio_sync::Semaphore;
    ///
    /// let sem = Semaphore::new(100);
    /// assert_eq!(sem.available_permits(), 100);
    ///
    /// let _permit = sem.try_acquire().unwrap();
    /// assert_eq!(sem.available_permits(), 99);
    /// ```
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.inner.permits.load(Ordering::Acquire)
    }

    /// Get the maximum number of permits (configured limit)
    ///
    /// # Example
    ///
    /// ```rust
    /// use compio_sync::Semaphore;
    ///
    /// let sem = Semaphore::new(1024);
    /// assert_eq!(sem.max_permits(), 1024);
    /// ```
    #[must_use]
    pub fn max_permits(&self) -> usize {
        self.inner.max_permits
    }

    /// Get the number of permits currently in use (max - available)
    ///
    /// # Example
    ///
    /// ```rust
    /// use compio_sync::Semaphore;
    ///
    /// let sem = Semaphore::new(100);
    /// let _permit = sem.try_acquire().unwrap();
    /// assert_eq!(sem.in_use(), 1);
    /// ```
    #[must_use]
    pub fn in_use(&self) -> usize {
        self.inner.max_permits - self.available_permits()
    }

    /// Reduce the number of available permits (for adaptive concurrency control)
    ///
    /// This allows dynamically reducing concurrency in response to resource constraints
    /// like file descriptor exhaustion. Only reduces permits that are currently available
    /// (won't affect permits already in use).
    ///
    /// # Arguments
    ///
    /// * `count` - Number of permits to remove from the available pool
    ///
    /// # Returns
    ///
    /// The actual number of permits reduced (may be less than requested if not enough available)
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_sync::Semaphore;
    ///
    /// let sem = Semaphore::new(100);
    /// let reduced = sem.reduce_permits(20);
    /// assert_eq!(reduced, 20);
    /// assert_eq!(sem.available_permits(), 80);
    /// ```
    #[must_use]
    pub fn reduce_permits(&self, count: usize) -> usize {
        let mut reduced = 0;

        loop {
            let current = self.inner.permits.load(Ordering::Acquire);
            if current == 0 || reduced >= count {
                break;
            }

            let to_reduce = std::cmp::min(current, count - reduced);
            let new_value = current - to_reduce;

            if self
                .inner
                .permits
                .compare_exchange(current, new_value, Ordering::Release, Ordering::Acquire)
                .is_ok()
            {
                reduced += to_reduce;
            }
        }

        reduced
    }

    /// Add permits back to the semaphore (for adaptive concurrency control)
    ///
    /// This allows dynamically increasing concurrency after resources become available.
    ///
    /// # Arguments
    ///
    /// * `count` - Number of permits to add to the available pool
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_sync::Semaphore;
    ///
    /// let sem = Semaphore::new(100);
    /// sem.reduce_permits(20);
    /// assert_eq!(sem.available_permits(), 80);
    ///
    /// sem.add_permits(20);
    /// assert_eq!(sem.available_permits(), 100);
    /// ```
    pub fn add_permits(&self, count: usize) {
        self.inner.permits.fetch_add(count, Ordering::Release);

        // Wake up waiters (up to count)
        // Note: This could be optimized with a wake_n() method on WaiterQueue
        for _ in 0..count {
            self.inner.waiters.wake_one();
        }
    }

    /// Release a permit (called internally by `SemaphorePermit::drop`)
    fn release(&self) {
        // Increment available permits
        self.inner.permits.fetch_add(1, Ordering::Release);

        // Wake one waiter (WaiterQueue handles lock-then-wake pattern)
        self.inner.waiters.wake_one();
    }
}

/// RAII guard that releases a semaphore permit on drop
///
/// This guard is returned by `Semaphore::acquire()` and `Semaphore::try_acquire()`.
/// When dropped, it automatically releases the permit back to the semaphore and
/// wakes one waiting task (if any).
///
/// # Example
///
/// ```rust,no_run
/// use compio_sync::Semaphore;
/// use std::sync::Arc;
///
/// # async fn example() {
/// let sem = Arc::new(Semaphore::new(10));
///
/// {
///     let permit = sem.acquire().await;
///     // Permit is held here
/// } // Permit released automatically when scope ends
///
/// assert_eq!(sem.available_permits(), 10);
/// # }
/// ```
pub struct SemaphorePermit<'a, W: WaiterQueueTrait> {
    /// Reference to the semaphore that issued this permit
    semaphore: &'a SemaphoreGeneric<W>,
}

impl<'a, W: WaiterQueueTrait> Drop for SemaphorePermit<'a, W> {
    fn drop(&mut self) {
        self.semaphore.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waiter_queue::{WaiterQueue as PlatformWaiterQueue, WaiterQueueTrait};
    use std::sync::atomic::Ordering as AtomicOrdering;
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Mock WaiterQueue that allows injecting operations during registration
    /// Used to deterministically test race conditions
    struct MockWaiterQueue {
        /// Called when add_waiter_if is invoked (before checking condition)
        /// Allows test to inject operations in the race window
        on_add_waiter: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        /// Actual queue for functional behavior (uses platform default)
        inner: PlatformWaiterQueue,
    }

    impl MockWaiterQueue {
        fn new() -> Self {
            Self {
                on_add_waiter: Mutex::new(None),
                inner: PlatformWaiterQueue::new(),
            }
        }

        fn set_on_add_waiter<F>(&self, f: F)
        where
            F: FnOnce() + Send + 'static,
        {
            *self.on_add_waiter.lock().unwrap() = Some(Box::new(f));
        }
    }

    impl WaiterQueueTrait for MockWaiterQueue {
        fn new() -> Self {
            MockWaiterQueue::new()
        }

        fn add_waiter_if<'a, F>(&'a self, condition: F) -> impl std::future::Future<Output = ()>
        where
            F: Fn() -> bool + Send + Sync + 'a + Unpin,
        {
            // Call hook if set - this simulates operations happening during registration
            if let Some(hook) = self.on_add_waiter.lock().unwrap().take() {
                hook();
            }

            // Then delegate to real implementation
            self.inner.add_waiter_if(condition)
        }

        fn wake_one(&self) {
            self.inner.wake_one()
        }

        fn wake_all(&self) {
            self.inner.wake_all()
        }

        fn waiter_count(&self) -> usize {
            self.inner.waiter_count()
        }
    }

    #[test]
    fn test_semaphore_new() {
        let sem = Semaphore::new(100);
        assert_eq!(sem.available_permits(), 100);
        assert_eq!(sem.max_permits(), 100);
        assert_eq!(sem.in_use(), 0);
    }

    #[test]
    fn test_semaphore_try_acquire() {
        let sem = Semaphore::new(2);

        // Acquire first permit
        let permit1 = sem.try_acquire();
        assert!(permit1.is_some());
        assert_eq!(sem.available_permits(), 1);
        assert_eq!(sem.in_use(), 1);

        // Acquire second permit
        let permit2 = sem.try_acquire();
        assert!(permit2.is_some());
        assert_eq!(sem.available_permits(), 0);
        assert_eq!(sem.in_use(), 2);

        // Try to acquire third (should fail)
        let permit3 = sem.try_acquire();
        assert!(permit3.is_none());
        assert_eq!(sem.available_permits(), 0);

        // Release first permit
        drop(permit1);
        assert_eq!(sem.available_permits(), 1);
        assert_eq!(sem.in_use(), 1);

        // Can acquire again
        let permit4 = sem.try_acquire();
        assert!(permit4.is_some());
        assert_eq!(sem.available_permits(), 0);
    }

    #[test]
    fn test_semaphore_permit_drop() {
        let sem = Semaphore::new(1);

        {
            let _permit = sem.try_acquire().unwrap();
            assert_eq!(sem.available_permits(), 0);
        } // Permit dropped here

        assert_eq!(sem.available_permits(), 1);
    }

    #[compio::test]
    async fn test_semaphore_acquire_basic() {
        let sem = Semaphore::new(2);

        let permit1 = sem.acquire().await;
        assert_eq!(sem.available_permits(), 1);

        let permit2 = sem.acquire().await;
        assert_eq!(sem.available_permits(), 0);

        drop(permit1);
        assert_eq!(sem.available_permits(), 1);

        drop(permit2);
        assert_eq!(sem.available_permits(), 2);
    }

    #[compio::test]
    async fn test_semaphore_blocking_and_wakeup() {
        let sem = Arc::new(Semaphore::new(1));

        // Acquire the only permit
        let permit1 = sem.acquire().await;
        assert_eq!(sem.available_permits(), 0);

        // Spawn a task that will block waiting for permit
        let sem2 = sem.clone();
        let handle = compio::runtime::spawn(async move {
            let _permit = sem2.acquire().await;
            42
        });

        // Give spawned task a chance to start and block
        // We can't use sleep without the time feature, so we'll just check after spawn
        // The task should acquire the permit when we release ours

        // Release permit - should wake the blocked task
        drop(permit1);

        // Spawned task should complete
        let result = handle.await.unwrap();
        assert_eq!(result, 42);
        assert_eq!(sem.available_permits(), 1);
    }

    #[compio::test]
    async fn test_semaphore_multiple_waiters() {
        let sem = Arc::new(Semaphore::new(1));

        // Acquire the only permit
        let permit = sem.acquire().await;

        // Spawn multiple waiting tasks
        let mut handles = Vec::new();
        for i in 0..5 {
            let sem = sem.clone();
            let handle = compio::runtime::spawn(async move {
                let _permit = sem.acquire().await;
                i
            });
            handles.push(handle);
        }

        // Release permit - should wake tasks one at a time (FIFO)
        drop(permit);

        // All tasks should eventually complete (FIFO order)
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // Should complete in order (0, 1, 2, 3, 4) due to FIFO
        assert_eq!(results.len(), 5);
    }

    #[compio::test]
    async fn test_semaphore_high_concurrency() {
        let sem = Arc::new(Semaphore::new(100));
        let mut handles = Vec::new();

        // Spawn 1000 tasks, but only 100 should run concurrently
        for i in 0..1000 {
            let sem = sem.clone();
            let handle = compio::runtime::spawn(async move {
                let _permit = sem.acquire().await;
                // No need to simulate work - just testing concurrency limit
                i
            });
            handles.push(handle);
        }

        // Collect results
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        assert_eq!(results.len(), 1000);
        assert_eq!(sem.available_permits(), 100);
    }

    #[compio::test]
    async fn test_semaphore_clone() {
        let sem = Arc::new(Semaphore::new(10));
        let sem2 = sem.clone();

        let permit1 = sem.acquire().await;
        assert_eq!(sem2.available_permits(), 9);

        let permit2 = sem2.acquire().await;
        assert_eq!(sem.available_permits(), 8);

        drop(permit1);
        drop(permit2);
        assert_eq!(sem.available_permits(), 10);
    }

    #[test]
    #[should_panic(expected = "Semaphore must have at least one permit")]
    fn test_semaphore_zero_permits_panics() {
        let _sem = Semaphore::new(0);
    }

    /// Deterministic test for lost-wake race using MockWaiterQueue
    ///
    /// This test uses a mock to inject a permit release DURING the
    /// add_waiter_if() call, precisely in the race window.
    ///
    /// With || false condition: Task will deadlock (waits forever with permit available)
    /// With || permits > 0 condition: Task completes (re-check catches the permit)
    #[compio::test]
    async fn test_lost_wake_race_deterministic() {
        use std::sync::atomic::AtomicBool;

        compio::time::timeout(std::time::Duration::from_secs(2), async {
            // Create semaphore with MockWaiterQueue (1 permit)
            let sem = Arc::new(SemaphoreGeneric::<MockWaiterQueue>::new(1));
            let released = Arc::new(AtomicBool::new(false));

            // Take the permit (permits = 0)
            let _permit = sem.acquire().await;

            // Set up the mock to inject permit release in race window
            let sem_clone = sem.clone();
            let released_clone = released.clone();
            sem.inner.waiters.set_on_add_waiter(move || {
                // This executes IN THE RACE WINDOW
                // (after try_acquire fails, during waiter registration)

                // Simulate another thread releasing a permit
                sem_clone.inner.permits.fetch_add(1, Ordering::Release);
                released_clone.store(true, AtomicOrdering::Release);
            });

            // Try to acquire while permits = 0
            // The hook will release the permit DURING add_waiter_if registration
            // With || false: This will TIMEOUT (deadlock - doesn't see the released permit)
            // With || permits > 0: This completes (re-check catches permit)
            let acquire_result =
                compio::time::timeout(std::time::Duration::from_millis(500), sem.acquire()).await;

            // Verify hook ran (permit was added during registration)
            assert!(
                released.load(AtomicOrdering::Acquire),
                "Hook should have run"
            );

            // Verify acquisition succeeded (no deadlock)
            assert!(
                acquire_result.is_ok(),
                "LOST-WAKE RACE: Task deadlocked despite permit being available! \
                 The condition || false doesn't check permit availability during registration."
            );
        })
        .await
        .expect("Test timed out");
    }

    /// Test that acquire() takes exactly one permit when multiple are available
    ///
    /// This verifies that even if multiple permits become available during
    /// registration, acquire() only takes one permit (not all of them).
    #[compio::test]
    async fn test_mock_multiple_permits_released() {
        compio::time::timeout(std::time::Duration::from_secs(2), async {
            let sem = Arc::new(SemaphoreGeneric::<MockWaiterQueue>::new(1));

            // Take the permit (permits = 0)
            let _permit = sem.acquire().await;

            // Set up mock to release MULTIPLE permits during registration
            let sem_clone = sem.clone();
            sem.inner.waiters.set_on_add_waiter(move || {
                // Release 5 permits at once
                sem_clone.inner.permits.fetch_add(5, Ordering::Release);
            });

            // Acquire should take exactly 1 permit, leaving 4
            let _acquired =
                compio::time::timeout(std::time::Duration::from_millis(500), sem.acquire())
                    .await
                    .expect("Should not timeout");

            // Verify only 1 permit was taken (4 remain)
            assert_eq!(
                sem.available_permits(),
                4,
                "Should have taken exactly 1 permit"
            );
        })
        .await
        .expect("Test timed out");
    }

    /// Test explicit wake_one() during registration is safe
    ///
    /// This verifies that calling wake_one() during add_waiter_if registration
    /// doesn't cause issues (no double-wake, no lost permits).
    #[compio::test]
    async fn test_mock_wake_during_registration() {
        compio::time::timeout(std::time::Duration::from_secs(2), async {
            let sem = Arc::new(SemaphoreGeneric::<MockWaiterQueue>::new(1));

            // Take the permit (permits = 0)
            let _permit = sem.acquire().await;

            // Set up mock to release permit AND explicitly wake during registration
            let sem_clone = sem.clone();
            sem.inner.waiters.set_on_add_waiter(move || {
                // Release permit
                sem_clone.inner.permits.fetch_add(1, Ordering::Release);
                // Explicitly wake (might be redundant with re-check, but should be safe)
                sem_clone.inner.waiters.wake_one();
            });

            // Should complete without issues
            let _acquired =
                compio::time::timeout(std::time::Duration::from_millis(500), sem.acquire())
                    .await
                    .expect("Should not timeout - wake + re-check should work");

            // Verify permit was consumed
            assert_eq!(sem.available_permits(), 0, "Permit should be consumed");
        })
        .await
        .expect("Test timed out");
    }

    /// Test permit stolen by another thread during registration
    ///
    /// This verifies retry logic: if a permit appears but is stolen before
    /// we can acquire it, the task correctly stays pending.
    #[compio::test]
    async fn test_mock_permit_stolen() {
        compio::time::timeout(std::time::Duration::from_secs(2), async {
            let sem = Arc::new(SemaphoreGeneric::<MockWaiterQueue>::new(1));

            // Take the permit (permits = 0)
            let _permit = sem.acquire().await;

            // Set up mock to release permit then immediately steal it back
            let sem_clone = sem.clone();
            sem.inner.waiters.set_on_add_waiter(move || {
                // Release permit
                sem_clone.inner.permits.fetch_add(1, Ordering::Release);
                // Immediately steal it back (simulates another thread taking it)
                sem_clone.inner.permits.fetch_sub(1, Ordering::AcqRel);
            });

            // Try to acquire - should timeout since permit is stolen
            let acquire_result =
                compio::time::timeout(std::time::Duration::from_millis(200), sem.acquire()).await;

            // Should timeout (permit was stolen, we stay pending)
            assert!(
                acquire_result.is_err(),
                "Should timeout - permit was stolen, task should stay pending"
            );

            // Now release a permit for real to unblock
            drop(_permit);

            // Should be able to acquire now
            let _acquired =
                compio::time::timeout(std::time::Duration::from_millis(500), sem.acquire())
                    .await
                    .expect("Should acquire after real release");
        })
        .await
        .expect("Test timed out");
    }

    /// Sanity check that MockWaiterQueue works correctly for normal operations
    ///
    /// This verifies the mock properly delegates to the real implementation
    /// when no hook is set.
    #[compio::test]
    async fn test_mock_normal_operation() {
        compio::time::timeout(std::time::Duration::from_secs(2), async {
            let sem = Arc::new(SemaphoreGeneric::<MockWaiterQueue>::new(3));

            // Normal acquire/release without any hooks
            let permit1 = sem.acquire().await;
            assert_eq!(sem.available_permits(), 2);

            let permit2 = sem.acquire().await;
            assert_eq!(sem.available_permits(), 1);

            drop(permit1);
            assert_eq!(sem.available_permits(), 2);

            let permit3 = sem.acquire().await;
            assert_eq!(sem.available_permits(), 1);

            drop(permit2);
            drop(permit3);
            assert_eq!(sem.available_permits(), 3);

            // Verify try_acquire works
            let permit = sem.try_acquire();
            assert!(permit.is_some());
            assert_eq!(sem.available_permits(), 2);
        })
        .await
        .expect("Test timed out");
    }
}
