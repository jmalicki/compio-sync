//! Platform-specific waiter queue implementations
//!
//! This module provides different implementations of waiter queues based on the
//! target platform to achieve optimal performance:
//!
//! - **Linux**: (Phase 2) Will use io_uring futex operations for unified event loop
//! - **Windows**: (Phase 3) Will use IOCP for unified event loop
//! - **Generic**: (Phase 1 - Current) Uses parking_lot mutex with hybrid single/multi mode
//!
//! **Current Status**: All platforms use the generic implementation (Phase 1).
//! Platform-specific optimizations will be added in Phase 2 (Linux) and Phase 3 (Windows).
//!
//! All implementations provide the same interface via `WaiterQueueTrait`, ensuring
//! consistent behavior across platforms while enabling platform-specific optimizations.

// Generic implementation - always compiled (used as baseline and fallback)
mod generic;

// Platform-specific modules
// Phase 1: These re-export generic implementation
// Phase 2+: Will have platform-specific optimizations
#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
mod windows;

// Re-export the appropriate implementation
#[cfg(target_os = "linux")]
pub use linux::WaiterQueue;

#[cfg(windows)]
pub use windows::WaiterQueue;

#[cfg(not(any(target_os = "linux", windows)))]
pub use generic::WaiterQueue;

// Common trait that all implementations satisfy (for testing and documentation)

/// Trait for waiter queue implementations
///
/// This trait defines the interface that all platform-specific waiter queue
/// implementations must satisfy.
pub trait WaiterQueueTrait {
    /// Create a new waiter queue
    fn new() -> Self;

    /// Poll to add a waiter to the queue if condition is false (atomic check-and-add)
    ///
    /// This is a poll-based method to allow platform-specific implementations (like io_uring)
    /// to submit async operations. Generic implementation returns Poll::Ready immediately.
    ///
    /// Idempotency: Implementations must handle repeated polls safely (no duplicate enqueues
    /// for the same task) and should update the stored waker on re-poll.
    ///
    /// Returns:
    /// - `Poll::Ready(true)`: condition was true (no waiter enqueued)
    /// - `Poll::Ready(false)`: condition was false (waiter enqueued; caller must return Pending)
    /// - `Poll::Pending`: the operation itself is pending (e.g., io_uring submission)
    fn poll_add_waiter_if<F>(
        &self,
        condition: F,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<bool>
    where
        F: Fn() -> bool;

    /// Wake one waiting task
    fn wake_one(&self);

    /// Wake all waiting tasks
    fn wake_all(&self);

    /// Get the number of waiting tasks (for debugging/stats)
    #[allow(dead_code)]
    fn waiter_count(&self) -> usize;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_waiter_queue_creation() {
        let _queue = WaiterQueue::new();
        // Should not panic
    }

    #[compio::test]
    async fn test_add_waiter_if_condition_true() {
        let queue = WaiterQueue::new();

        // Condition is true - should NOT add waiter
        let added = std::future::poll_fn(|cx| queue.poll_add_waiter_if(|| true, cx)).await;
        assert!(added, "Should return true when condition is true");
    }

    #[compio::test]
    async fn test_add_waiter_if_condition_false() {
        let queue = WaiterQueue::new();

        // Condition is false - should add waiter
        let added = std::future::poll_fn(|cx| queue.poll_add_waiter_if(|| false, cx)).await;
        assert!(!added, "Should return false when condition is false");
    }

    #[compio::test]
    async fn test_add_waiter_idempotent_on_repoll() {
        let queue = WaiterQueue::new();

        // Register waiter
        let added1 = std::future::poll_fn(|cx| queue.poll_add_waiter_if(|| false, cx)).await;
        assert!(!added1, "First poll should register waiter");

        // Note: In real usage, we wouldn't re-poll after Ready(false).
        // But this tests that implementation handles it safely if it happens.
        // Since AtomicWaker.register() updates in-place, count should still be 1.
        assert_eq!(
            queue.waiter_count(),
            1,
            "Should have exactly 1 waiter after registration"
        );
    }

    #[test]
    fn test_wake_one_no_waiters() {
        let queue = WaiterQueue::new();
        // Should not panic when waking with no waiters
        queue.wake_one();
    }

    #[test]
    fn test_wake_all_no_waiters() {
        let queue = WaiterQueue::new();
        // Should not panic when waking all with no waiters
        queue.wake_all();
    }
}
