//! Integration tests for Semaphore

use compio_sync::Semaphore;
use std::sync::Arc;
use std::time::Duration;

#[compio::test]
async fn test_semaphore_basic_acquire_release() {
    compio::time::timeout(Duration::from_secs(5), async {
        let sem = Semaphore::new(1);
        let permit = sem.acquire().await;
        assert_eq!(sem.available_permits(), 0);
        drop(permit);
        assert_eq!(sem.available_permits(), 1);
    })
    .await
    .expect("test timed out");
}

#[compio::test]
async fn test_semaphore_concurrent_access() {
    compio::time::timeout(Duration::from_secs(10), async {
        let sem = Arc::new(Semaphore::new(5));
        let mut handles = vec![];

        // Spawn 20 tasks, but only 5 can run concurrently
        for i in 0..20 {
            let sem = sem.clone();
            let handle = compio::runtime::spawn(async move {
                let _permit = sem.acquire().await;
                // Small delay to ensure concurrency
                compio::time::sleep(Duration::from_millis(10)).await;
                i
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for (i, handle) in handles.into_iter().enumerate() {
            assert_eq!(handle.await.unwrap(), i);
        }

        // All permits should be released
        assert_eq!(sem.available_permits(), 5);
    })
    .await
    .expect("test timed out");
}

#[compio::test]
async fn test_semaphore_try_acquire() {
    compio::time::timeout(Duration::from_secs(5), async {
        let sem = Semaphore::new(1);

        let permit1 = sem.try_acquire();
        assert!(permit1.is_some());
        assert_eq!(sem.available_permits(), 0);

        let permit2 = sem.try_acquire();
        assert!(permit2.is_none());

        drop(permit1);
        assert_eq!(sem.available_permits(), 1);

        let permit3 = sem.try_acquire();
        assert!(permit3.is_some());
    })
    .await
    .expect("test timed out");
}

#[compio::test]
async fn test_semaphore_multiple_permits() {
    compio::time::timeout(Duration::from_secs(5), async {
        let sem = Arc::new(Semaphore::new(10));

        // Acquire 5 permits
        let mut permits = vec![];
        for _ in 0..5 {
            permits.push(sem.acquire().await);
        }

        assert_eq!(sem.available_permits(), 5);
        assert_eq!(sem.in_use(), 5);
        assert_eq!(sem.max_permits(), 10);

        // Release 2 permits
        permits.pop();
        permits.pop();

        assert_eq!(sem.available_permits(), 7);
        assert_eq!(sem.in_use(), 3);
    })
    .await
    .expect("test timed out");
}

#[compio::test]
async fn test_semaphore_single_permit() {
    compio::time::timeout(Duration::from_secs(5), async {
        // Semaphore requires at least one permit
        let sem = Arc::new(Semaphore::new(1));

        assert_eq!(sem.available_permits(), 1);
        assert_eq!(sem.max_permits(), 1);

        // Acquire the only permit
        let permit = sem.acquire().await;
        assert_eq!(sem.available_permits(), 0);

        // Try to acquire should fail
        assert!(sem.try_acquire().is_none());

        // Spawn task that will wait for the permit
        let sem_clone = sem.clone();
        let handle = compio::runtime::spawn(async move {
            let _permit = sem_clone.acquire().await;
            "acquired"
        });

        // Give it time to start waiting
        compio::time::sleep(Duration::from_millis(10)).await;

        // Release the permit
        drop(permit);

        // Task should complete now
        let result = compio::time::timeout(Duration::from_millis(100), handle).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().unwrap(), "acquired");
    })
    .await
    .expect("test timed out");
}

#[compio::test]
async fn test_semaphore_fairness() {
    compio::time::timeout(Duration::from_secs(10), async {
        let sem = Arc::new(Semaphore::new(1));
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Hold the semaphore
        let permit = sem.acquire().await;

        // Spawn 5 waiters
        let mut handles = vec![];
        for i in 0..5 {
            let sem = sem.clone();
            let order = order.clone();
            let handle = compio::runtime::spawn(async move {
                let _permit = sem.acquire().await;
                order.lock().unwrap().push(i);
            });
            handles.push(handle);
        }

        // Give them time to queue up
        compio::time::sleep(Duration::from_millis(50)).await;

        // Release the permit
        drop(permit);

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Check they ran in order (FIFO)
        let final_order = order.lock().unwrap();
        assert_eq!(*final_order, vec![0, 1, 2, 3, 4]);
    })
    .await
    .expect("test timed out");
}

#[compio::test]
async fn test_semaphore_stress() {
    compio::time::timeout(Duration::from_secs(10), async {
        let sem = Arc::new(Semaphore::new(100));
        let mut handles = vec![];

        // Spawn 1000 tasks competing for 100 permits
        for i in 0..1000 {
            let sem = sem.clone();
            let handle = compio::runtime::spawn(async move {
                let _permit = sem.acquire().await;
                // Minimal work
                i * 2
            });
            handles.push(handle);
        }

        // All should complete successfully
        for (i, handle) in handles.into_iter().enumerate() {
            assert_eq!(handle.await.unwrap(), i * 2);
        }

        // All permits released
        assert_eq!(sem.available_permits(), 100);
    })
    .await
    .expect("test timed out");
}

#[compio::test]
async fn test_semaphore_api_methods() {
    compio::time::timeout(Duration::from_secs(5), async {
        let sem = Semaphore::new(50);

        assert_eq!(sem.max_permits(), 50);
        assert_eq!(sem.available_permits(), 50);
        assert_eq!(sem.in_use(), 0);

        let _permit1 = sem.acquire().await;
        assert_eq!(sem.available_permits(), 49);
        assert_eq!(sem.in_use(), 1);

        let _permit2 = sem.acquire().await;
        assert_eq!(sem.available_permits(), 48);
        assert_eq!(sem.in_use(), 2);
    })
    .await
    .expect("test timed out");
}
