# compio-sync

Async synchronization primitives for the [compio](https://github.com/compio-rs/compio) runtime.

## Features

- **Semaphore**: Async semaphore for bounding concurrency
  - Lock-free fast path using atomics
  - FIFO waiter queue for fairness
  - RAII permit guards for automatic cleanup
  - Compatible with compio's async runtime

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
compio-sync = "0.1"
```

### Semaphore Example

```rust
use compio_sync::Semaphore;
use std::sync::Arc;

#[compio::main]
async fn main() {
    // Create semaphore with 100 permits
    let sem = Arc::new(Semaphore::new(100));
    
    // Spawn 1000 tasks, but only 100 run concurrently
    let mut handles = Vec::new();
    for i in 0..1000 {
        let sem = sem.clone();
        let handle = compio::runtime::spawn(async move {
            let _permit = sem.acquire().await;
            println!("Task {} running (max 100 concurrent)", i);
            // Permit automatically released when dropped
        });
        handles.push(handle);
    }
    
    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }
}
```

## Semaphore API

```rust
impl Semaphore {
    /// Create a new semaphore with the given number of permits
    pub fn new(permits: usize) -> Self;
    
    /// Acquire a permit, waiting asynchronously if none available
    pub async fn acquire(&self) -> SemaphorePermit;
    
    /// Try to acquire a permit without waiting
    pub fn try_acquire(&self) -> Option<SemaphorePermit>;
    
    /// Get the number of available permits
    pub fn available_permits(&self) -> usize;
    
    /// Get the maximum number of permits (configured limit)
    pub fn max_permits(&self) -> usize;
    
    /// Get the number of permits currently in use
    pub fn in_use(&self) -> usize;
}
```

## Design

The semaphore uses a two-tier approach for optimal performance:

1. **Fast Path (Lock-Free)**: When permits are available, acquisition uses atomic compare-and-swap operations with no mutex locking
2. **Slow Path (Mutex-Protected)**: When no permits are available, tasks register their waker in a FIFO queue and wait for notification

This design minimizes contention while providing fair scheduling.

## Use Cases

- **Bounding Concurrency**: Limit the number of concurrent file operations
- **Resource Protection**: Prevent file descriptor or memory exhaustion  
- **Backpressure**: Pause discovery when processing is saturated
- **Rate Limiting**: Control operation throughput

## Comparison with Tokio

This semaphore is inspired by [`tokio::sync::Semaphore`](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html) but built specifically for the compio runtime:

| Feature | tokio::sync::Semaphore | compio-sync::Semaphore |
|---------|----------------------|----------------------|
| Runtime | Tokio | Compio |
| Lock-free fast path | ✅ | ✅ |
| FIFO fairness | ✅ | ✅ |
| RAII permits | ✅ | ✅ |
| Async acquire | ✅ | ✅ |
| Try acquire | ✅ | ✅ |

## Development & Research

### Lock-Free Wakeup Research

We are actively researching and planning to eliminate mutexes from our synchronization primitives to achieve truly lock-free async wakeups. This work is documented in the [`docs/`](./docs/) directory:

- **[Implementation Plan](./docs/implementation-plan-lockfree.md)** - Step-by-step guide for implementing lock-free wakeups
- **[Research Document](./docs/mutex-free-wakeup-research.md)** - Comprehensive analysis of approaches (Tokio, Python asyncio, etc.)
- **[Visual Comparison](./docs/wakeup-approaches-comparison.md)** - Side-by-side code comparisons and decision matrix
- **[Docs Index](./docs/README.md)** - Complete documentation guide

### Planned Improvements

**Phase 1: parking_lot + AtomicWaker** (1-2 days)
- Replace `std::sync::Mutex` with `parking_lot::Mutex`
- Add `AtomicWaker` fast path for single-waiter scenarios
- Expected: 20-30% performance improvement

**Phase 2: crossbeam Lock-Free Queue** (1 week)
- Replace mutex-based queue with `crossbeam-queue::SegQueue`
- True lock-free operation
- Expected: 40-60% performance improvement

**Phase 3: Intrusive Lists** (Optional, 3-4 weeks)
- Tokio-style intrusive linked lists
- Zero allocations, maximum performance
- Only if Phase 2 isn't sufficient

See the [research docs](./docs/) for detailed analysis and trade-offs.

## CI Matrix

Our CI tests across multiple platforms to ensure compatibility and performance:

| Platform | Purpose | Implementation |
|----------|--------|----------------|
| **Ubuntu 24.04** | Modern Linux (kernel 6.11) | io_uring futex integration |
| **Ubuntu 22.04** | Older Linux (kernel 5.15) | Generic fallback (parking_lot) |
| **Windows 2022** | Modern Windows | IOCP event integration (planned) |

### Platform-Specific Features

- **Linux**: Uses io_uring futex operations for unified event loop
- **Windows**: Will use IOCP events for unified event loop (Phase 3)
- **Fallback**: Generic implementation using parking_lot + AtomicWaker

## Contributing

Contributions are welcome! Please see:

- [Design documents](./docs/) for architecture and research
- [Semaphore Design](./docs/semaphore-design.md) for current implementation
- [Implementation Plan](./docs/implementation-plan-lockfree.md) for planned improvements

## License

MIT

