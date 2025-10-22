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

    /// Add a waiter to the queue if condition is false (atomic check-and-add)
    ///
    /// Completes when either:
    /// - Condition was already true (fast path, no registration)
    /// - Waiter was registered and subsequently woken
    ///
    /// After awaiting, caller should re-check the actual condition.
    ///
    /// For io_uring, can return submit() future directly.
    /// For generic, returns immediately-ready future.
    ///
    /// **Note**: The returned future is `!Send` because io_uring operations are
    /// thread-local. This is fine for compio's single-threaded runtime model.
    fn add_waiter_if<'a, F>(&'a self, condition: F) -> impl std::future::Future<Output = ()>
    where
        F: Fn() -> bool + Send + Sync + 'a;

    /// Wake one waiting task
    ///
    /// **Ordering**: Wake order is implementation-dependent and NOT guaranteed to be FIFO.
    /// - Generic: FIFO (uses parking_lot queue)
    /// - io_uring: Unspecified (kernel scheduling)
    fn wake_one(&self);

    /// Wake all waiting tasks
    ///
    /// **Ordering**: Wake order is implementation-dependent and NOT guaranteed to be FIFO.
    /// All waiters will be woken, but in an unspecified order.
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

        // Condition is true - should complete immediately
        queue.add_waiter_if(|| true).await;
        // If we got here, the future completed (which is correct for true condition)
    }

    #[compio::test]
    async fn test_add_waiter_if_condition_false() {
        let queue = std::sync::Arc::new(WaiterQueue::new());
        let queue_clone = queue.clone();

        // Condition is false - will register and pend
        // We need to wake it to complete
        let handle = compio::runtime::spawn(async move {
            queue_clone.add_waiter_if(|| false).await;
        });

        // Give it time to register
        compio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Wake it
        queue.wake_one();

        // Should complete now
        compio::time::timeout(std::time::Duration::from_millis(100), handle)
            .await
            .expect("Should complete after wake")
            .expect("Task should succeed");
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
