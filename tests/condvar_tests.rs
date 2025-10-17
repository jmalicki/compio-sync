//! Integration tests for Condvar

use compio_sync::Condvar;
use std::sync::Arc;
use std::time::Duration;

#[compio::test]
async fn test_condvar_basic_wait_notify() {
    let cv = Arc::new(Condvar::new());
    let cv_clone = cv.clone();
    
    let handle = compio::runtime::spawn(async move {
        cv_clone.wait().await;
        42
    });
    
    // Give waiter time to start
    compio::time::sleep(Duration::from_millis(10)).await;
    
    // Notify
    cv.notify_all();
    
    // Verify completion
    assert_eq!(handle.await.unwrap(), 42);
}

#[compio::test]
async fn test_condvar_multiple_waiters() {
    let cv = Arc::new(Condvar::new());
    let mut handles = vec![];
    
    // Spawn 10 waiters
    for i in 0..10 {
        let cv = cv.clone();
        let handle = compio::runtime::spawn(async move {
            cv.wait().await;
            i
        });
        handles.push(handle);
    }
    
    // Give waiters time to start
    compio::time::sleep(Duration::from_millis(10)).await;
    
    // Notify all
    cv.notify_all();
    
    // Verify all complete
    for (i, handle) in handles.into_iter().enumerate() {
        assert_eq!(handle.await.unwrap(), i);
    }
}

#[compio::test]
async fn test_condvar_notify_one() {
    let cv = Arc::new(Condvar::new());
    let completed = Arc::new(std::sync::Mutex::new(Vec::new()));
    
    // Spawn 5 waiters
    let mut handles = vec![];
    for i in 0..5 {
        let cv = cv.clone();
        let completed = completed.clone();
        let handle = compio::runtime::spawn(async move {
            cv.wait().await;
            completed.lock().unwrap().push(i);
        });
        handles.push(handle);
    }
    
    // Give them time to start waiting
    compio::time::sleep(Duration::from_millis(10)).await;
    
    // Notify one at a time
    for _ in 0..5 {
        cv.notify_one();
        compio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    // All should have completed
    let final_completed = completed.lock().unwrap();
    assert_eq!(final_completed.len(), 5);
}

#[compio::test]
async fn test_condvar_notify_before_wait() {
    let cv = Arc::new(Condvar::new());
    
    // Notify before anyone is waiting
    cv.notify_all();
    
    // Subsequent wait should complete immediately
    let cv_clone = cv.clone();
    let handle = compio::runtime::spawn(async move {
        cv_clone.wait().await;
        "done"
    });
    
    // Should complete quickly
    let result = compio::time::timeout(Duration::from_millis(100), handle).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap(), "done");
}

#[compio::test]
async fn test_condvar_producer_consumer() {
    let cv = Arc::new(Condvar::new());
    let ready = Arc::new(std::sync::Mutex::new(false));
    let data = Arc::new(std::sync::Mutex::new(Vec::new()));
    
    // Consumer task
    let cv_consumer = cv.clone();
    let ready_consumer = ready.clone();
    let data_consumer = data.clone();
    let consumer = compio::runtime::spawn(async move {
        // Wait for data to be ready
        cv_consumer.wait().await;
        
        // Check data is ready
        assert!(*ready_consumer.lock().unwrap());
        
        // Consume data
        let consumed = data_consumer.lock().unwrap().clone();
        consumed
    });
    
    // Give consumer time to start waiting
    compio::time::sleep(Duration::from_millis(10)).await;
    
    // Producer: prepare data
    {
        let mut d = data.lock().unwrap();
        d.push(1);
        d.push(2);
        d.push(3);
        *ready.lock().unwrap() = true;
    }
    
    // Notify consumer
    cv.notify_all();
    
    // Verify consumer got the data
    let result = consumer.await.unwrap();
    assert_eq!(result, vec![1, 2, 3]);
}

#[compio::test]
async fn test_condvar_stress() {
    let cv = Arc::new(Condvar::new());
    let mut handles = vec![];
    
    // Spawn 100 waiters
    for i in 0..100 {
        let cv = cv.clone();
        let handle = compio::runtime::spawn(async move {
            cv.wait().await;
            i * 2
        });
        handles.push(handle);
    }
    
    // Give them time to start waiting
    compio::time::sleep(Duration::from_millis(50)).await;
    
    // Notify all at once
    cv.notify_all();
    
    // All should complete
    for (i, handle) in handles.into_iter().enumerate() {
        assert_eq!(handle.await.unwrap(), i * 2);
    }
}

#[compio::test]
async fn test_condvar_shared_via_arc() {
    // Condvar should be wrapped in Arc for sharing
    let cv = Arc::new(Condvar::new());
    
    let cv_clone = cv.clone();
    let handle = compio::runtime::spawn(async move {
        cv_clone.wait().await;
        "notified"
    });
    
    // Give waiter time to start
    compio::time::sleep(Duration::from_millis(10)).await;
    
    // Notify via original Arc
    cv.notify_all();
    
    // Should wake up the waiter
    let result = compio::time::timeout(Duration::from_millis(100), handle).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap(), "notified");
}

#[compio::test]
async fn test_condvar_sequential_notifications() {
    let cv = Arc::new(Condvar::new());
    let count = Arc::new(std::sync::Mutex::new(0));
    
    // First waiter
    let cv1 = cv.clone();
    let count1 = count.clone();
    let handle1 = compio::runtime::spawn(async move {
        cv1.wait().await;
        *count1.lock().unwrap() += 1;
    });
    
    compio::time::sleep(Duration::from_millis(10)).await;
    cv.notify_all();
    handle1.await.unwrap();
    
    // Second waiter (after first completed)
    let cv2 = cv.clone();
    let count2 = count.clone();
    let handle2 = compio::runtime::spawn(async move {
        cv2.wait().await;
        *count2.lock().unwrap() += 1;
    });
    
    compio::time::sleep(Duration::from_millis(10)).await;
    cv.notify_all();
    handle2.await.unwrap();
    
    // Both should have incremented
    assert_eq!(*count.lock().unwrap(), 2);
}

