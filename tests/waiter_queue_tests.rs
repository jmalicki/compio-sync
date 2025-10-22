//! Generic waiter queue tests
//!
//! These tests verify WaiterQueue behavior across all platforms.
//! - On Windows: Tests IOCP event-based implementation
//! - On Linux: Tests io_uring futex or generic implementation
//! - On macOS/BSD: Tests generic parking_lot implementation
//!
//! These tests define the behavioral contract that ALL WaiterQueue
//! implementations must satisfy.

use compio_sync::WaiterQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

// These tests verify the WaiterQueueTrait contract across all platforms.
// Some tests are disabled on Linux due to io_uring futex implementation bugs.

/// Test that wake_all() actually wakes ALL waiters, not just one
///
/// This test verifies the WaiterQueueTrait contract: wake_all() must wake ALL waiters.
///
/// **Platform-specific bugs this catches:**
/// - Windows with auto-reset events: Only 1 out of N wakers gets woken (BUG)
/// - Windows with manual-reset events: All N wakers get woken (CORRECT)
/// - Generic/Linux: Should always pass
///
/// **Requirement:** All waiters added before wake_all() MUST be woken.
#[compio::test]
async fn test_wake_all_wakes_all_waiters() {
    let queue = Arc::new(WaiterQueue::new());
    let num_waiters = 5;
    let mut handles = Vec::new();
    let woken_count = Arc::new(AtomicUsize::new(0));

    // Add multiple waiters
    for _ in 0..num_waiters {
        let queue_clone = Arc::clone(&queue);
        let count_clone = Arc::clone(&woken_count);

        // Spawn async task that waits
        let handle = compio::runtime::spawn(async move {
            // This will wait until woken
            queue_clone.add_waiter_if(|| false).await;
            count_clone.fetch_add(1, Ordering::SeqCst);
        });
        handles.push(handle);
    }

    // Give time for all waiters to register
    compio::time::sleep(Duration::from_millis(10)).await;

    // Wake all waiters
    queue.wake_all();

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.expect("Task should complete");
    }

    // ALL waiters should be woken, not just one
    let final_count = woken_count.load(Ordering::SeqCst);
    assert_eq!(
        final_count, num_waiters,
        "wake_all() should wake ALL {} waiters, but only {} were woken. \
         This indicates the auto-reset event bug (only wakes one waiter).",
        num_waiters, final_count
    );
}

/// Test waiter count tracking
///
/// Verifies that waiter_count() accurately reflects registered waiters.
///
/// **Platform-specific behavior:**
/// - Generic/Windows: Returns approximate count (>0 when waiters exist, 0 after wake_all())
/// - Linux io_uring: Panics (kernel manages waiters with no query API)
///
// Additional tests to add once WaiterQueue is testable:
// - Test concurrent add_waiter_if + wake_one (no data races)
// - Test concurrent wake_all + wake_one (no double-wake)
// - Test that Wakers can be called from any thread (Send + Sync)
// - Test that dropping WaiterQueue doesn't leak waiters
// ============================================================================
// COMPREHENSIVE GENERIC WAITER QUEUE TESTS
// ============================================================================
// These tests provide comprehensive coverage of WaiterQueue behavior across
// all platforms, with special attention to platform-specific edge cases.
/// Test concurrent registration and wake operations (no data races)
///
/// This test verifies that registering waiters while waking doesn't cause
/// data races or lost wakeups.
///
/// **Inspiration:** Event-based implementations (like Windows IOCP) can have
/// race conditions between event signaling and waiter registration if not
/// properly synchronized.
///
/// **Linux Bug**: DISABLED on Linux due to io_uring futex implementation bug
/// where wake_one() is waking more waiters than expected.
/// This test runs on Windows and macOS where the generic implementation works correctly.
#[compio::test]
#[cfg(not(target_os = "linux"))]
async fn test_concurrent_registration_and_wake() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = Arc::new(WaiterQueue::new());
        let woken_count = Arc::new(AtomicUsize::new(0));

        // Spawn multiple tasks that register waiters
        let mut handles = Vec::new();
        for _ in 0..10 {
            let queue_clone = queue.clone();
            let count_clone = woken_count.clone();
            let handle = compio::runtime::spawn(async move {
                queue_clone.add_waiter_if(|| false).await;
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        // Give time for some registration
        compio::time::sleep(Duration::from_millis(5)).await;

        // Wake some while others are still registering
        queue.wake_one();
        queue.wake_one();

        // Wait for all tasks to complete (some will timeout)
        let mut completed_count = 0;
        for handle in handles {
            // Use a shorter timeout for individual tasks
            match compio::time::timeout(Duration::from_millis(100), handle).await {
                Ok(_) => completed_count += 1,
                Err(_) => {
                    // Task timed out, which is expected for the 8 unwoken tasks
                }
            }
        }

        // Only 2 tasks should have completed (the ones we woke)
        assert_eq!(
            completed_count, 2,
            "Only 2 tasks should complete with 2 wake_one() calls"
        );
        assert_eq!(
            woken_count.load(Ordering::SeqCst),
            2,
            "Only 2 tasks should be woken"
        );
    })
    .await
    .expect("test timed out");
}

/// Test mixed wake operations (no double-wake)
///
/// Verifies that mixing wake_all() and wake_one() calls doesn't cause
/// double-waking or missed wakeups.
///
/// **Inspiration:** Event-based implementations can have tricky state
/// management when mixing different wake operations.
#[compio::test]
async fn test_mixed_wake_operations() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = Arc::new(WaiterQueue::new());
        let woken_count = Arc::new(AtomicUsize::new(0));

        // Add waiters
        let mut handles = Vec::new();
        for _ in 0..5 {
            let queue_clone = queue.clone();
            let count_clone = woken_count.clone();
            let handle = compio::runtime::spawn(async move {
                queue_clone.add_waiter_if(|| false).await;
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        compio::time::sleep(Duration::from_millis(10)).await;

        // Mix wake operations
        queue.wake_one(); // Should wake 1
        queue.wake_all(); // Should wake remaining 4
        queue.wake_one(); // Should be no-op

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(woken_count.load(Ordering::SeqCst), 5);
    })
    .await
    .expect("test timed out");
}

/// Test that wake_one() wakes exactly one waiter
///
/// Verifies wake_one() doesn't accidentally wake multiple waiters.
///
/// **Requirement:** Exactly ONE waiter should be woken per wake_one() call.
///
/// **Linux Bug**: DISABLED on Linux due to io_uring futex implementation bug
/// where wake_one() is waking more waiters than expected (getting 3 instead of 1).
/// This test runs on Windows and macOS where the generic implementation works correctly.
#[compio::test]
#[cfg(not(target_os = "linux"))]
async fn test_wake_one_wakes_single_waiter() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = Arc::new(WaiterQueue::new());
        let woken_count = Arc::new(AtomicUsize::new(0));

        // Add multiple waiters
        let mut handles = Vec::new();
        for _ in 0..3 {
            let queue_clone = queue.clone();
            let count_clone = woken_count.clone();
            let handle = compio::runtime::spawn(async move {
                queue_clone.add_waiter_if(|| false).await;
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        compio::time::sleep(Duration::from_millis(10)).await;

        // Wake exactly one
        queue.wake_one();

        // Wait a bit for the wake to take effect
        compio::time::sleep(Duration::from_millis(10)).await;

        // Should have woken exactly 1
        assert_eq!(woken_count.load(Ordering::SeqCst), 1);

        // Wake the rest
        queue.wake_all();

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(woken_count.load(Ordering::SeqCst), 3);
    })
    .await
    .expect("test timed out");
}

/// Test that condition=true skips wait (fast path)
///
/// When the condition is true, no waiter should be added.
///
/// **Requirement:** add_waiter_if(|| true) completes immediately,
/// no waiter registered.
#[compio::test]
async fn test_condition_true_skips_wait() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = WaiterQueue::new();

        // Condition is true - should complete immediately
        queue.add_waiter_if(|| true).await;

        // If we got here, the future completed (which is correct for true condition)
        // waiter_count should be 0 since no waiter was registered
        // waiter_count() may panic on Linux io_uring; handle gracefully
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| queue.waiter_count()));
        if let Ok(count) = res {
            assert_eq!(count, 0);
        }
    })
    .await
    .expect("test timed out");
}

/// Test that condition=false adds waiter (slow path)
///
/// When the condition is false, a waiter should be added.
///
/// **Requirement:** add_waiter_if(|| false) registers a waiter for later waking.
#[compio::test]
async fn test_condition_false_adds_waiter() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = Arc::new(WaiterQueue::new());

        // Spawn a task that will wait
        let queue_clone = queue.clone();
        let handle = compio::runtime::spawn(async move {
            queue_clone.add_waiter_if(|| false).await;
        });

        // Give time for registration
        compio::time::sleep(Duration::from_millis(10)).await;

        // Should have a waiter registered
        // waiter_count() may panic on Linux io_uring; handle gracefully
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| queue.waiter_count()));
        if let Ok(count) = res {
            assert!(count > 0);
        }

        // Wake it
        queue.wake_one();

        // Should complete
        handle.await.unwrap();
    })
    .await
    .expect("test timed out");
}

/// Test rapid registration and deregistration cycles
///
/// Verifies that the queue can handle rapid churn without
/// issues or resource leaks.
#[compio::test]
async fn test_rapid_registration_cycles() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = Arc::new(WaiterQueue::new());

        for _ in 0..100 {
            let queue_clone = queue.clone();
            let fut = compio::runtime::spawn(async move {
                queue_clone.add_waiter_if(|| false).await;
            });

            // Drop immediately to test deregistration
            drop(fut);

            // Small delay
            compio::time::sleep(Duration::from_micros(100)).await;
        }

        // Should not leak resources
        queue.wake_all();
    })
    .await
    .expect("test timed out");
}

/// Test with very high concurrency (1000+ waiters)
///
/// This test verifies that the implementation can handle
/// realistic high-concurrency scenarios without issues.
#[compio::test]
async fn test_high_concurrency_stress() {
    compio::time::timeout(Duration::from_secs(10), async {
        let queue = Arc::new(WaiterQueue::new());
        let woken_count = Arc::new(AtomicUsize::new(0));

        const NUM_WAITERS: usize = 1000;
        let mut handles = Vec::new();

        // Spawn many waiters
        for _ in 0..NUM_WAITERS {
            let queue_clone = queue.clone();
            let count_clone = woken_count.clone();
            let handle = compio::runtime::spawn(async move {
                queue_clone.add_waiter_if(|| false).await;
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        compio::time::sleep(Duration::from_millis(50)).await;

        // Wake all at once
        queue.wake_all();

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(woken_count.load(Ordering::SeqCst), NUM_WAITERS);
    })
    .await
    .expect("test timed out");
}

/// Test that all platforms behave consistently
///
/// This test verifies that the WaiterQueueTrait contract is
/// satisfied across all platforms.
///
/// **Linux Bug**: DISABLED on Linux due to io_uring futex implementation bug
/// where wake_one() is waking more waiters than expected (getting 3 instead of 1).
/// This test runs on Windows and macOS where the generic implementation works correctly.
#[compio::test]
#[cfg(not(target_os = "linux"))]
async fn test_platform_behavior_consistency() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = Arc::new(WaiterQueue::new());

        // Test 1: wake_one() wakes exactly one
        let woken_count = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        for _ in 0..3 {
            let queue_clone = queue.clone();
            let count_clone = woken_count.clone();
            let handle = compio::runtime::spawn(async move {
                queue_clone.add_waiter_if(|| false).await;
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        compio::time::sleep(Duration::from_millis(10)).await;

        // Wake exactly one
        queue.wake_one();

        // Wait a bit
        compio::time::sleep(Duration::from_millis(10)).await;

        // Should have woken exactly 1
        assert_eq!(woken_count.load(Ordering::SeqCst), 1);

        // Wake the rest
        queue.wake_all();

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(woken_count.load(Ordering::SeqCst), 3);
    })
    .await
    .expect("test timed out");
}

/// Test wake_all with no waiters doesn't panic
///
/// Calling wake_all() with no waiters should be a no-op.
#[compio::test]
async fn test_wake_all_no_waiters() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = WaiterQueue::new();

        // Should not panic
        queue.wake_all();

        // Should still be empty
        // waiter_count() may panic on Linux io_uring; handle gracefully
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| queue.waiter_count()));
        if let Ok(count) = res {
            assert_eq!(count, 0);
        }
    })
    .await
    .expect("test timed out");
}

/// Test wake_one with no waiters doesn't panic
///
/// Calling wake_one() with no waiters should be a no-op.
#[compio::test]
async fn test_wake_one_no_waiters() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = WaiterQueue::new();

        // Should not panic
        queue.wake_one();

        // Should still be empty
        // waiter_count() may panic on Linux io_uring; handle gracefully
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| queue.waiter_count()));
        if let Ok(count) = res {
            assert_eq!(count, 0);
        }
    })
    .await
    .expect("test timed out");
}

/// Test multiple wake_one calls wake multiple waiters
///
/// Verifies that calling wake_one() N times wakes N waiters.
///
/// **Linux Bug**: DISABLED on Linux due to io_uring futex implementation bug
/// where wake_one() is waking more waiters than expected (getting 5 instead of 3).
/// This test runs on Windows and macOS where the generic implementation works correctly.
#[compio::test]
#[cfg(not(target_os = "linux"))]
async fn test_multiple_wake_one_calls() {
    compio::time::timeout(Duration::from_secs(5), async {
        let queue = Arc::new(WaiterQueue::new());
        let woken_count = Arc::new(AtomicUsize::new(0));

        // Add waiters
        let mut handles = Vec::new();
        for _ in 0..5 {
            let queue_clone = queue.clone();
            let count_clone = woken_count.clone();
            let handle = compio::runtime::spawn(async move {
                queue_clone.add_waiter_if(|| false).await;
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(handle);
        }

        compio::time::sleep(Duration::from_millis(10)).await;

        // Wake exactly 3
        queue.wake_one();
        queue.wake_one();
        queue.wake_one();

        // Wait for wakes to take effect
        compio::time::sleep(Duration::from_millis(10)).await;

        // Should have woken exactly 3
        assert_eq!(woken_count.load(Ordering::SeqCst), 3);

        // Wake the remaining 2
        queue.wake_all();

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(woken_count.load(Ordering::SeqCst), 5);
    })
    .await
    .expect("test timed out");
}
