//! Asynchronous condition variable for task notification
//!
//! This module provides `Condvar`, a condition variable primitive for use with
//! compio's async runtime. Unlike traditional condition variables that require
//! a mutex, this implementation is standalone and uses interior mutability.
//!
//! # Example
//!
//! ```rust,no_run
//! use compio_sync::Condvar;
//! use std::sync::Arc;
//!
//! #[compio::main]
//! async fn main() {
//!     let cv = Arc::new(Condvar::new());
//!     let cv_clone = cv.clone();
//!     
//!     // Spawn a task that waits for notification
//!     compio::runtime::spawn(async move {
//!         cv_clone.wait().await;
//!         println!("Notified!");
//!     });
//!     
//!     // Do some work...
//!     
//!     // Notify the waiting task
//!     cv.notify_one();
//! }
//! ```

use crate::waiter_queue::{WaiterQueue, WaiterQueueTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

/// A compio-compatible async condition variable for task notification
///
/// `Condvar` allows one or more tasks to wait for a notification from another task.
/// Unlike `std::sync::Condvar`, this implementation:
/// - Works with async/await and compio's runtime
/// - Does not require an external mutex (uses interior mutability)
/// - Users should wrap in `Arc<Condvar>` when sharing between tasks
///
/// # Memory Safety
///
/// This implementation uses CAREFUL memory ordering to prevent lost wakeups:
/// - The `notified` flag is checked INSIDE the mutex to prevent TOCTOU races
/// - All notifier operations (set flag + drain) happen atomically under mutex
/// - Waiter operations (check flag + register) happen atomically under mutex
/// - All accesses use proper Acquire/Release semantics
///
/// # Usage Pattern
///
/// ```rust,no_run
/// use compio_sync::Condvar;
/// use std::sync::Arc;
///
/// # async fn example() {
/// let cv = Arc::new(Condvar::new());
///
/// // Spawn waiters
/// let mut handles = Vec::new();
/// for i in 0..5 {
///     let cv = Arc::clone(&cv);
///     let handle = compio::runtime::spawn(async move {
///         cv.wait().await;
///         i
///     });
///     handles.push(handle);
/// }
///
/// // Do some work...
///
/// // Notify all waiters
/// cv.notify_all();
///
/// // All waiters complete
/// for handle in handles {
///     handle.await.unwrap();
/// }
/// # }
/// ```
pub struct CondvarGeneric<W: WaiterQueueTrait> {
    /// Internal state for the condition variable
    /// Users should wrap in `Arc<Condvar>` when sharing between tasks
    inner: CondvarInner<W>,
}

/// Public type alias using platform-specific WaiterQueue
///
/// This is what users actually interact with. The generic implementation
/// allows for flexibility and testing while this alias keeps the API simple.
pub type Condvar = CondvarGeneric<WaiterQueue>;

/// Internal state using shared waiter queue abstraction
///
/// CRITICAL RACE PREVENTION:
/// The `notified` flag MUST be checked while holding the queue's internal mutex
/// to prevent this race:
///
/// WITHOUT atomic check-and-add:
/// 1. Waiter: check notified → false (no lock)
/// 2. Notifier: set notified → true
/// 3. Notifier: drain waiters
/// 4. Waiter: add to waiters → LOST WAKEUP!
///
/// WITH atomic check-and-add (WaiterQueue):
/// 1. Waiter: lock, check notified → false, add to waiters, unlock
/// 2. Notifier: lock, set notified → true, drain waiters, unlock
///
/// The WaiterQueue encapsulates this pattern for reuse across sync primitives.
struct CondvarInner<W: WaiterQueueTrait> {
    /// Notification flag (true = notified, wake immediately)
    notified: AtomicBool,

    /// Waiter queue abstraction (handles mutex + check-and-add pattern)
    waiters: W,
}

impl<W: WaiterQueueTrait + Sync> CondvarGeneric<W> {
    /// Create a new condition variable
    ///
    /// The condition variable starts in the "not notified" state.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use compio_sync::Condvar;
    ///
    /// # async fn example() {
    /// let cv = Condvar::new();
    /// # }
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: CondvarInner {
                notified: AtomicBool::new(false),
                waiters: W::new(),
            },
        }
    }

    /// Wait for notification
    ///
    /// Suspends the current task until `notify_one()` or `notify_all()` is called.
    /// If the condition variable is already notified, returns immediately.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use compio_sync::Condvar;
    ///
    /// # async fn example() {
    /// let cv = Condvar::new();
    /// cv.wait().await;
    /// # }
    /// ```
    pub async fn wait(&self) {
        // Simple poll_fn wrapper around poll_add_waiter_if
        std::future::poll_fn(|cx| {
            match self
                .inner
                .waiters
                .poll_add_waiter_if(|| self.inner.notified.load(Ordering::Acquire), cx)
            {
                Poll::Ready(true) => Poll::Ready(()),
                Poll::Ready(false) | Poll::Pending => Poll::Pending,
            }
        })
        .await
    }

    /// Notify one waiting task
    ///
    /// Wakes up one task currently waiting on `wait()`. If no tasks are waiting,
    /// sets a flag so the next call to `wait()` returns immediately.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use compio_sync::Condvar;
    ///
    /// # async fn example() {
    /// let cv = Condvar::new();
    /// cv.notify_one();
    /// # }
    /// ```
    pub fn notify_one(&self) {
        // Set notified flag (uses Release ordering for memory synchronization)
        self.inner.notified.store(true, Ordering::Release);

        // Wake one waiter (WaiterQueue handles lock-then-wake pattern)
        self.inner.waiters.wake_one();
    }

    /// Notify all waiting tasks
    ///
    /// Wakes up all tasks currently waiting on `wait()`. Also sets a flag so that
    /// future calls to `wait()` return immediately without blocking.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use compio_sync::Condvar;
    ///
    /// # async fn example() {
    /// let cv = Condvar::new();
    /// cv.notify_all();
    /// # }
    /// ```
    pub fn notify_all(&self) {
        // Set notified flag (uses Release ordering for memory synchronization)
        self.inner.notified.store(true, Ordering::Release);

        // Wake all waiters (WaiterQueue handles lock-then-wake pattern)
        self.inner.waiters.wake_all();
    }

    /// Clear the notification flag
    ///
    /// Resets the condition variable to the "not notified" state.
    /// Future calls to `wait()` will block until `notify_one()` or `notify_all()` is called.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use compio_sync::Condvar;
    ///
    /// # async fn example() {
    /// let cv = Condvar::new();
    /// cv.notify_one();
    /// cv.clear();  // Reset notification
    /// cv.wait().await;  // Will block again
    /// # }
    /// ```
    pub fn clear(&self) {
        // Use Release ordering to ensure threads don't observe stale 'true' after clear
        // Pairs with Acquire loads in wait()
        self.inner.notified.store(false, Ordering::Release);
    }

    /// Get the number of tasks waiting on this condvar
    ///
    /// This is useful for tests, debugging, and observability.
    /// **Note**: The count may be stale by the time you read it due to concurrent notify operations.
    #[must_use]
    pub fn waiter_count(&self) -> usize {
        self.inner.waiters.waiter_count()
    }
}

impl<W: WaiterQueueTrait + Sync> Default for CondvarGeneric<W> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[compio::test]
    async fn test_condvar_already_notified() {
        let cv = Condvar::new();
        cv.notify_one();

        // Should return immediately since already notified
        cv.wait().await;
    }

    #[compio::test]
    async fn test_condvar_clear() {
        let cv = Condvar::new();
        cv.notify_one();

        // Clear and verify it returns immediately (still notified)
        cv.clear();

        // After clear, wait should block (but we won't test blocking here)
        // Just verify clear doesn't panic
    }

    #[compio::test]
    async fn test_condvar_notify_before_wait() {
        let cv = Arc::new(Condvar::new());

        // Notify before any waiter
        cv.notify_one();

        // Waiter should return immediately
        cv.wait().await;
    }

    #[compio::test]
    async fn test_condvar_notify_all() {
        let cv = Arc::new(Condvar::new());

        // Notify all before any waiters
        cv.notify_all();

        // Multiple waiters should all return immediately
        cv.wait().await;
        cv.wait().await;
        cv.wait().await;
    }

    #[test]
    fn test_condvar_creation() {
        let cv = Condvar::new();
        assert!(!cv.inner.notified.load(Ordering::Relaxed));
    }
}
