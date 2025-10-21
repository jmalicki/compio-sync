//! Linux-specific tests for io_uring futex integration

#![cfg(target_os = "linux")]

use compio_sync::Semaphore;
use std::sync::Arc;

#[compio::test]
async fn test_linux_semaphore_basic() {
    use std::time::Duration;

    // Basic test that should work whether using futex or fallback
    let sem = Semaphore::new(1);

    // Add timeout to prevent hanging
    let result = compio::time::timeout(Duration::from_secs(5), async {
        let permit = sem.acquire().await;
        assert_eq!(sem.available_permits(), 0);

        drop(permit);
        assert_eq!(sem.available_permits(), 1);
    })
    .await;

    assert!(
        result.is_ok(),
        "Test timed out - futex might not be working"
    );
}

#[compio::test]
async fn test_linux_high_concurrency() {
    // Test that exercises the unified event loop
    let sem = Arc::new(Semaphore::new(10));
    let mut handles = vec![];

    for i in 0..100 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            let _p = sem.acquire().await;
            i
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(sem.available_permits(), 10);
}

#[compio::test]
async fn test_linux_futex_wake_all() {
    let sem = Arc::new(Semaphore::new(1));

    // Hold the permit
    let permit = sem.acquire().await;

    // Spawn multiple waiters
    let mut handles = vec![];
    for i in 0..10 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            let _p = sem.acquire().await;
            i
        }));
    }

    // Release - should wake waiters one by one
    drop(permit);

    // All should complete
    for h in handles {
        h.await.unwrap();
    }
}

#[test]
fn test_kernel_version_detection() {
    // This test will show what implementation is being used
    let sem = Semaphore::new(1);

    // Try to acquire - this should work regardless of implementation
    let permit = sem.try_acquire();
    assert!(permit.is_some());

    // Print debug info about what's being used
    println!("Semaphore created successfully");
    println!("Available permits: {}", sem.available_permits());

    // Check if we're on a new enough kernel
    if let Ok(version_str) = std::fs::read_to_string("/proc/version") {
        println!(
            "Kernel: {}",
            version_str.lines().next().unwrap_or("unknown")
        );
    }
}

#[compio::test]
async fn test_linux_mixed_io_and_sync() {
    // This test mixes I/O operations with synchronization
    // On Linux with io_uring futex, both go through the same event loop

    let sem = Arc::new(Semaphore::new(5));
    let mut handles = vec![];

    for i in 0..20 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            // Acquire permit (sync primitive)
            let _p = sem.acquire().await;

            // Do some I/O (file operation)
            // This goes through io_uring
            let _ = compio::fs::File::open("/proc/version").await;

            i
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(sem.available_permits(), 5);
}
