//! Generic waiter queue tests
//!
//! These tests verify WaiterQueue behavior across all platforms.
//! - On Windows: Tests IOCP event-based implementation
//! - On Linux: Tests io_uring futex or generic implementation
//! - On macOS/BSD: Tests generic parking_lot implementation

use compio_sync::waiter_queue::{WaiterQueue, WaiterQueueTrait};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

/// Custom waker for testing
struct TestWaker {
    woken: Arc<AtomicBool>,
}

impl Wake for TestWaker {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::Release);
    }
}

impl TestWaker {
    fn new() -> (Waker, Arc<AtomicBool>) {
        let woken = Arc::new(AtomicBool::new(false));
        let waker = Arc::new(TestWaker {
            woken: Arc::clone(&woken),
        })
        .into();
        (waker, woken)
    }
}

/// Test that wake_all() actually wakes ALL waiters, not just one
///
/// This test verifies the fix for the auto-reset event bug.
/// With auto-reset events (CreateEventW param = 0), only ONE waiter gets woken.
/// With manual-reset events (CreateEventW param = 1) + ResetEvent, ALL waiters get woken.
///
/// **Expected behavior with bug (FAILS):**
/// - Only 1 out of 5 wakers gets woken
///
/// **Expected behavior with fix (PASSES):**
/// - All 5 wakers get woken
#[test]
fn test_wake_all_wakes_all_waiters() {
    let queue = WaiterQueue::new();
    let num_waiters = 5;
    let mut wakers = Vec::new();
    let mut woken_flags = Vec::new();

    // Add multiple waiters
    for _ in 0..num_waiters {
        let (waker, woken) = TestWaker::new();
        
        // Add waiter with a condition that's always false (so they all wait)
        let registered = queue.add_waiter_if(|| false, waker);
        assert!(!registered, "Waiter should be pending (condition is false)");
        
        woken_flags.push(woken);
    }

    // Wake all waiters
    queue.wake_all();

    // Give a small delay for async notifications
    std::thread::sleep(Duration::from_millis(10));

    // Count how many were actually woken
    let woken_count = woken_flags
        .iter()
        .filter(|flag| flag.load(Ordering::Acquire))
        .count();

    // ALL waiters should be woken, not just one
    assert_eq!(
        woken_count, num_waiters,
        "wake_all() should wake ALL {} waiters, but only {} were woken. \
         This indicates the auto-reset event bug (only wakes one waiter).",
        num_waiters, woken_count
    );
}

/// Test that wake_one() wakes exactly one waiter
///
/// This verifies that wake_one() doesn't accidentally wake multiple waiters.
#[test]
fn test_wake_one_wakes_single_waiter() {
    let queue = WaiterQueue::new();
    let num_waiters = 5;
    let mut wakers = Vec::new();
    let mut woken_flags = Vec::new();

    // Add multiple waiters
    for _ in 0..num_waiters {
        let (waker, woken) = TestWaker::new();
        
        let registered = queue.add_waiter_if(|| false, waker);
        assert!(!registered, "Waiter should be pending");
        
        woken_flags.push(woken);
    }

    // Wake ONE waiter
    queue.wake_one();

    // Give a small delay for async notifications
    std::thread::sleep(Duration::from_millis(10));

    // Count how many were actually woken
    let woken_count = woken_flags
        .iter()
        .filter(|flag| flag.load(Ordering::Acquire))
        .count();

    // Exactly ONE waiter should be woken
    // Note: With auto-reset events this works correctly
    // With manual-reset events, we need special handling to wake only one
    assert_eq!(
        woken_count, 1,
        "wake_one() should wake exactly 1 waiter, but {} were woken",
        woken_count
    );
}

/// Test waiter count tracking
///
/// Verifies that the waiter count is correctly maintained.
#[test]
fn test_waiter_count_tracking() {
    let queue = WaiterQueue::new();
    
    assert_eq!(queue.waiter_count(), 0, "Should start with 0 waiters");

    // Add 3 waiters
    for _ in 0..3 {
        let (waker, _) = TestWaker::new();
        queue.add_waiter_if(|| false, waker);
    }

    // Note: waiter_count is approximate and implementation-dependent
    // For IOCP events, we track this manually
    let count = queue.waiter_count();
    assert!(
        count > 0,
        "Should have waiters registered (got {})",
        count
    );

    // Wake all
    queue.wake_all();

    // After wake_all, count should be 0
    assert_eq!(
        queue.waiter_count(),
        0,
        "After wake_all(), waiter count should be reset to 0"
    );
}

/// Test that condition check works correctly
///
/// Verifies that when the condition is true, no waiter is added.
#[test]
fn test_condition_true_skips_wait() {
    let queue = WaiterQueue::new();
    let (waker, woken) = TestWaker::new();

    // Condition is true - should NOT add waiter
    let registered = queue.add_waiter_if(|| true, waker);

    assert!(
        registered,
        "When condition is true, should return true (no wait needed)"
    );
    assert_eq!(
        queue.waiter_count(),
        0,
        "No waiter should be added when condition is true"
    );
    assert!(
        !woken.load(Ordering::Acquire),
        "Waker should not be woken (no wait happened)"
    );
}

/// Test that condition check works correctly when false
///
/// Verifies that when the condition is false, a waiter is added.
#[test]
fn test_condition_false_adds_waiter() {
    let queue = WaiterQueue::new();
    let (waker, _) = TestWaker::new();

    // Condition is false - should add waiter
    let registered = queue.add_waiter_if(|| false, waker);

    assert!(
        !registered,
        "When condition is false, should return false (wait needed)"
    );
    assert!(
        queue.waiter_count() > 0,
        "A waiter should be added when condition is false"
    );
}

/// Stress test: Many waiters with wake_all
///
/// Verifies behavior with a larger number of waiters.
#[test]
fn test_wake_all_many_waiters() {
    let queue = WaiterQueue::new();
    let num_waiters = 100;
    let woken_count = Arc::new(AtomicUsize::new(0));

    // Add many waiters
    for _ in 0..num_waiters {
        let count_clone = Arc::clone(&woken_count);
        let waker = Arc::new(TestWaker {
            woken: Arc::new(AtomicBool::new(false)),
        })
        .into();
        
        // Custom wake that increments counter
        struct CountingWaker {
            count: Arc<AtomicUsize>,
        }
        impl Wake for CountingWaker {
            fn wake(self: Arc<Self>) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
        }
        
        let counting_waker: Waker = Arc::new(CountingWaker {
            count: count_clone,
        })
        .into();
        
        queue.add_waiter_if(|| false, counting_waker);
    }

    // Wake all
    queue.wake_all();

    // Give time for all wakes to propagate
    std::thread::sleep(Duration::from_millis(50));

    // All waiters should have been woken
    let final_count = woken_count.load(Ordering::SeqCst);
    assert_eq!(
        final_count, num_waiters,
        "wake_all() should wake all {} waiters, but only {} were woken",
        num_waiters, final_count
    );
}

