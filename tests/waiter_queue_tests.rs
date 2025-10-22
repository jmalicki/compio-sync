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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Wake, Waker};
use std::time::Duration;

/// Custom waker for testing
#[allow(dead_code)]
struct TestWaker {
    woken: Arc<AtomicBool>,
}

impl Wake for TestWaker {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::Release);
    }
}

impl TestWaker {
    #[allow(dead_code)]
    fn create() -> (Waker, Arc<AtomicBool>) {
        let woken = Arc::new(AtomicBool::new(false));
        let waker = Arc::new(TestWaker {
            woken: Arc::clone(&woken),
        })
        .into();
        (waker, woken)
    }
}

/// Counting waker for stress tests
#[allow(dead_code)]
struct CountingWaker {
    count: Arc<AtomicUsize>,
}

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

impl CountingWaker {
    #[allow(dead_code)]
    fn create() -> (Waker, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let waker = Arc::new(CountingWaker {
            count: Arc::clone(&count),
        })
        .into();
        (waker, count)
    }
}

// NOTE: These tests are currently disabled because WaiterQueue is not
// publicly exposed. They will be enabled once we add #[cfg(test)] visibility
// or create a test-only API.

// The tests below document the required behavior for ALL WaiterQueue implementations.

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
    compio::time::sleep(std::time::Duration::from_millis(10)).await;

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
/// Test should handle the panic case for io_uring or be skipped on that platform.
#[test]
#[ignore = "WaiterQueue not yet exposed for testing - will be enabled in implementation PR"]
fn test_waiter_count_tracking() {
    // Expected behavior:
    // Generic/Windows:
    //   1. Start: waiter_count() == 0
    //   2. Add 3 waiters: waiter_count() > 0
    //   3. wake_all(): waiter_count() == 0
    // Linux io_uring:
    //   - waiter_count() panics (no kernel query API)
    //   - Consider #[cfg] gating or catch_unwind for this platform
}



/// Stress test: Many waiters with wake_all
///
/// Verifies behavior with a large number of waiters (100+).
///
/// **This is the most important test for the Windows auto-reset bug.**
/// With auto-reset events, only 1 out of 100 waiters would be woken.
#[test]
#[ignore = "WaiterQueue not yet exposed for testing - will be enabled in implementation PR"]
fn test_wake_all_many_waiters() {
    // Expected behavior:
    // 1. Add 100 waiters
    // 2. Call wake_all()
    // 3. Verify all 100 were woken
    //
    // This is a CRITICAL test for Windows IOCP implementation.
}




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
#[compio::test]
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
        
        // Wait for all
        for handle in handles {
            handle.await.unwrap();
        }
        
        assert_eq!(woken_count.load(Ordering::SeqCst), 10);
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
        queue.wake_one();  // Should wake 1
        queue.wake_all();  // Should wake remaining 4
        queue.wake_one();  // Should be no-op
        
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
#[compio::test]
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
        #[cfg(not(target_os = "linux"))]
        assert_eq!(queue.waiter_count(), 0);
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
        #[cfg(not(target_os = "linux"))]
        assert!(queue.waiter_count() > 0);
        
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
#[compio::test]
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
        #[cfg(not(target_os = "linux"))]
        assert_eq!(queue.waiter_count(), 0);
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
        #[cfg(not(target_os = "linux"))]
        assert_eq!(queue.waiter_count(), 0);
    })
    .await
    .expect("test timed out");
}

/// Test multiple wake_one calls wake multiple waiters
///
/// Verifies that calling wake_one() N times wakes N waiters.
#[compio::test]
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
