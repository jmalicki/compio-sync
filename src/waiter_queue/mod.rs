//! Platform-specific waiter queue implementations
//!
//! This module provides different implementations of waiter queues based on the
//! target platform to achieve optimal performance:
//!
//! - **Linux**: Uses io_uring futex operations for unified event loop
//! - **Windows**: Uses IOCP for unified event loop
//! - **Generic**: Uses lock-free queue or mutex for other platforms
//!
//! All implementations provide the same interface, ensuring consistent behavior
//! across platforms while leveraging platform-specific optimizations.

// Generic implementation - always compiled (used as baseline and fallback)
mod generic;

// Platform-specific modules (currently stub implementations that re-export generic)
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
use std::task::Waker;

/// Trait for waiter queue implementations
///
/// This trait defines the interface that all platform-specific waiter queue
/// implementations must satisfy.
#[allow(dead_code)]
pub trait WaiterQueueTrait {
    /// Create a new waiter queue
    fn new() -> Self;

    /// Add a waiter to the queue if condition is false (atomic check-and-add)
    ///
    /// Returns:
    /// - `true` if condition was true (don't wait)
    /// - `false` if condition was false and waiter was added (pending)
    fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: FnOnce() -> bool;

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
    use std::sync::Arc;
    use std::task::Wake;

    // Helper to create a dummy waker for testing
    struct DummyWaker;
    impl Wake for DummyWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn dummy_waker() -> Waker {
        Arc::new(DummyWaker).into()
    }

    #[test]
    fn test_waiter_queue_creation() {
        let _queue = WaiterQueue::new();
        // Should not panic
    }

    #[test]
    fn test_add_waiter_if_condition_true() {
        let queue = WaiterQueue::new();
        let waker = dummy_waker();

        // Condition is true - should NOT add waiter
        let added = queue.add_waiter_if(|| true, waker);
        assert!(added, "Should return true when condition is true");
    }

    #[test]
    fn test_add_waiter_if_condition_false() {
        let queue = WaiterQueue::new();
        let waker = dummy_waker();

        // Condition is false - should add waiter
        let added = queue.add_waiter_if(|| false, waker);
        assert!(!added, "Should return false when condition is false");
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
