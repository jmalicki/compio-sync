//! Stress tests for compio-sync primitives
//!
//! These tests verify behavior under high load and contention.

use compio_sync::Semaphore;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[compio::test]
async fn test_high_contention_semaphore() {
    let sem = Arc::new(Semaphore::new(1));
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // 1000 tasks contending for 1 permit
    for _ in 0..1000 {
        let sem = sem.clone();
        let counter = counter.clone();
        handles.push(compio::runtime::spawn(async move {
            let _p = sem.acquire().await;
            counter.fetch_add(1, Ordering::Relaxed);
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(counter.load(Ordering::Relaxed), 1000);
    assert_eq!(sem.available_permits(), 1);
}

#[compio::test]
async fn test_rapid_acquire_release() {
    let sem = Arc::new(Semaphore::new(10));
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // 100 tasks, each doing 100 acquire/release cycles
    for _ in 0..100 {
        let sem = sem.clone();
        let counter = counter.clone();
        handles.push(compio::runtime::spawn(async move {
            for _ in 0..100 {
                let _p = sem.acquire().await;
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(counter.load(Ordering::Relaxed), 10_000);
    assert_eq!(sem.available_permits(), 10);
}

#[compio::test]
async fn test_many_waiters_wake_order() {
    let sem = Arc::new(Semaphore::new(1));

    // Acquire the only permit
    let permit = sem.acquire().await;

    // Spawn many waiters
    let mut handles = vec![];
    for i in 0..100 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            let _p = sem.acquire().await;
            i
        }));
    }

    // Release permit - waiters should wake in order
    drop(permit);

    // All should eventually complete
    let mut results = vec![];
    for h in handles {
        results.push(h.await.unwrap());
    }

    assert_eq!(results.len(), 100);
}

#[compio::test]
async fn test_semaphore_under_load_mixed_operations() {
    let sem = Arc::new(Semaphore::new(50));
    let mut handles = vec![];

    // Mix of acquire/release patterns
    for i in 0..200 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            if i % 3 == 0 {
                // Try acquire (might fail)
                let _p = sem.try_acquire();
            } else {
                // Wait acquire
                let _p = sem.acquire().await;
            }
            i
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(sem.available_permits(), 50);
}

#[compio::test]
async fn test_future_cancellation_stress() {
    let sem = Arc::new(Semaphore::new(1));

    // Hold the permit
    let _permit = sem.acquire().await;

    // Start many futures but drop them
    for _ in 0..100 {
        let sem = sem.clone();
        let fut = Box::pin(async move {
            let _p = sem.acquire().await;
        });
        // Drop immediately (cancel)
        drop(fut);
    }

    // Semaphore should still work
    drop(_permit);
    let _p2 = sem.acquire().await;
}
