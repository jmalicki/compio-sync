# Detailed Implementation and Testing Plan: Platform-Specific Lock-Free Wakeups

**Version:** 1.0  
**Date:** 2025-10-21  
**Status:** Ready for Implementation

---

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Implementation Phases](#implementation-phases)
4. [Platform-Specific Details](#platform-specific-details)
5. [Testing Strategy](#testing-strategy)
6. [CI/CD Configuration](#cicd-configuration)
7. [Benchmarking Plan](#benchmarking-plan)
8. [Timeline and Milestones](#timeline-and-milestones)
9. [Risk Management](#risk-management)
10. [Success Criteria](#success-criteria)

---

## Overview

### Goal

Implement platform-specific wakeup mechanisms for compio-sync that:
- Use unified event loops on Linux (io_uring) and Windows (IOCP)
- Maintain lock-free fallback for other platforms
- Keep identical API across all platforms
- Ensure comprehensive cross-platform testing

### Three-Tier Architecture

| Platform | Implementation | Event Loop | Complexity |
|----------|---------------|------------|------------|
| **Linux** | io_uring futex | Unified | Medium |
| **Windows** | IOCP events | Unified | Medium |
| **Generic** | Lock-free queue | Dual | Low |

---

## Architecture

### Directory Structure

```
compio-sync/
├── Cargo.toml
├── README.md
├── CHANGELOG.md
├── .github/
│   └── workflows/
│       ├── ci.yml              # Main CI workflow
│       ├── benchmarks.yml      # Performance testing
│       └── coverage.yml        # Code coverage
├── docs/
│   ├── mutex-free-wakeup-research.md
│   ├── IMPLEMENTATION_PLAN_DETAILED.md  # This file
│   └── benchmarks/
│       └── results/            # Benchmark results by platform
├── src/
│   ├── lib.rs                  # Public API
│   ├── semaphore.rs            # Semaphore (platform-agnostic)
│   ├── condvar.rs              # Condvar (platform-agnostic)
│   └── waiter_queue/
│       ├── mod.rs              # Platform selection
│       ├── linux.rs            # Linux: io_uring futex
│       ├── windows.rs          # Windows: IOCP
│       └── generic.rs          # Generic: lock-free queue
├── tests/
│   ├── common/
│   │   └── mod.rs              # Shared test utilities
│   ├── semaphore_tests.rs      # Cross-platform semaphore tests
│   ├── condvar_tests.rs        # Cross-platform condvar tests
│   ├── linux_specific.rs       # Linux-only tests
│   ├── windows_specific.rs     # Windows-only tests
│   └── stress/
│       ├── high_contention.rs
│       ├── cancellation.rs
│       └── edge_cases.rs
├── benches/
│   ├── semaphore.rs            # Semaphore benchmarks
│   ├── condvar.rs              # Condvar benchmarks
│   └── platform_comparison.rs # Cross-platform comparison
└── examples/
    ├── basic_semaphore.rs
    ├── bounded_tasks.rs
    └── condvar_signaling.rs
```

---

## Implementation Phases

### Phase 1: Generic Implementation (Baseline)

**Timeline:** Week 1-2  
**Goal:** Get working implementation on all platforms

#### Tasks

1. **Refactor Current Code**
   ```
   [ ] Extract WaiterQueue trait/interface
   [ ] Move current implementation to generic.rs
   [ ] Update Semaphore to use trait-based WaiterQueue
   [ ] Update Condvar to use trait-based WaiterQueue
   ```

2. **Implement Generic WaiterQueue**
   ```
   [ ] Option A: parking_lot::Mutex + AtomicWaker fast path
   [ ] Option B: crossbeam-queue::SegQueue (lock-free)
   [ ] Add comprehensive tests
   [ ] Benchmark both options
   [ ] Choose best performer as default
   ```

3. **Set Up Platform Selection**
   ```
   [ ] Create src/waiter_queue/mod.rs with cfg logic
   [ ] Ensure it compiles on Linux, Windows, macOS
   [ ] Add feature flags if needed
   ```

#### Deliverables

- ✅ Generic implementation works on all platforms
- ✅ All existing tests pass
- ✅ Baseline benchmarks recorded
- ✅ Documentation updated

---

### Phase 2: Linux Implementation (io_uring futex)

**Timeline:** Week 3-4  
**Goal:** Unified event loop on Linux

#### Tasks

1. **Research io_uring Futex API**
   ```
   [ ] Study io_uring futex operations (IORING_OP_FUTEX_WAIT/WAKE)
   [ ] Review kernel requirements (Linux 6.7+)
   [ ] Understand integration with compio runtime
   [ ] Document API design
   ```

2. **Implement Linux WaiterQueue**
   ```rust
   // src/waiter_queue/linux.rs
   [ ] Struct definition with UringHandle
   [ ] Fast path: try_acquire with atomic CAS
   [ ] Slow path: submit_futex_wait
   [ ] Wake: submit_futex_wake
   [ ] Handle edge cases (runtime not available, old kernel)
   ```

3. **Integration with compio**
   ```
   [ ] Get reference to compio's io_uring instance
   [ ] Handle submission queue full (retry logic)
   [ ] Handle completion processing
   [ ] Error handling and fallback
   ```

4. **Testing**
   ```
   [ ] Basic functionality tests
   [ ] Integration with compio runtime
   [ ] Verify unified event loop
   [ ] Edge cases (old kernel, no io_uring)
   [ ] Stress tests
   ```

#### Deliverables

- ✅ Linux implementation complete
- ✅ Tests pass on Linux
- ✅ Fallback to generic if io_uring unavailable
- ✅ Benchmarks show unified event loop benefit
- ✅ Documentation with kernel requirements

#### Code Sketch

```rust
// src/waiter_queue/linux.rs
#[cfg(target_os = "linux")]
pub struct WaiterQueue {
    /// Handle to compio's io_uring instance
    uring: Arc<UringHandle>,
}

#[cfg(target_os = "linux")]
impl WaiterQueue {
    pub fn new(uring: Arc<UringHandle>) -> Self {
        Self { uring }
    }
    
    /// Wait on atomic value (futex wait)
    pub async fn wait(&self, addr: &AtomicUsize, expected: usize) {
        // Submit IORING_OP_FUTEX_WAIT to io_uring
        self.uring.submit_futex_wait(addr, expected).await
    }
    
    /// Wake one waiter (futex wake)
    pub fn wake_one(&self, addr: &AtomicUsize) {
        // Submit IORING_OP_FUTEX_WAKE to io_uring
        self.uring.submit_futex_wake(addr, 1);
    }
    
    /// Wake all waiters
    pub fn wake_all(&self, addr: &AtomicUsize) {
        // Submit IORING_OP_FUTEX_WAKE with INT_MAX
        self.uring.submit_futex_wake(addr, i32::MAX);
    }
}
```

---

### Phase 3: Windows Implementation (IOCP)

**Timeline:** Week 5-6  
**Goal:** Unified event loop on Windows

#### Tasks

1. **Research Windows IOCP API**
   ```
   [ ] Study NtAssociateWaitCompletionPacket (Windows 8+)
   [ ] Study PostQueuedCompletionStatus
   [ ] Understand integration with compio runtime
   [ ] Document API design
   ```

2. **Implement Windows WaiterQueue**
   ```rust
   // src/waiter_queue/windows.rs
   [ ] Struct definition with IocpHandle
   [ ] Fast path: try_acquire with atomic CAS
   [ ] Slow path: IOCP wait
   [ ] Wake: PostQueuedCompletionStatus
   [ ] Handle edge cases
   ```

3. **Integration with compio**
   ```
   [ ] Get reference to compio's IOCP instance
   [ ] Handle completion port full
   [ ] Handle completion processing
   [ ] Error handling and fallback
   ```

4. **Testing**
   ```
   [ ] Basic functionality tests
   [ ] Integration with compio runtime
   [ ] Verify unified event loop
   [ ] Edge cases
   [ ] Stress tests
   ```

#### Deliverables

- ✅ Windows implementation complete
- ✅ Tests pass on Windows
- ✅ Fallback to generic if IOCP unavailable
- ✅ Benchmarks show unified event loop benefit
- ✅ Documentation with Windows version requirements

#### Code Sketch

```rust
// src/waiter_queue/windows.rs
#[cfg(windows)]
pub struct WaiterQueue {
    /// Handle to compio's IOCP instance
    iocp: Arc<IocpHandle>,
}

#[cfg(windows)]
impl WaiterQueue {
    pub fn new(iocp: Arc<IocpHandle>) -> Self {
        Self { iocp }
    }
    
    /// Wait on atomic value
    pub async fn wait(&self, addr: &AtomicUsize) {
        // Create event and associate with IOCP
        let event = Event::new();
        self.iocp.associate_wait_completion(event);
        // Wait for completion
        self.iocp.wait_for_completion().await;
    }
    
    /// Wake one waiter
    pub fn wake_one(&self) {
        // Post custom completion to IOCP
        self.iocp.post_completion_status(WAKE_ONE_TOKEN, 0);
    }
    
    /// Wake all waiters
    pub fn wake_all(&self, count: usize) {
        // Post multiple completions
        for _ in 0..count {
            self.iocp.post_completion_status(WAKE_ALL_TOKEN, 0);
        }
    }
}
```

---

### Phase 4: Optimization and Polish

**Timeline:** Week 7-8  
**Goal:** Optimize and finalize

#### Tasks

1. **Performance Tuning**
   ```
   [ ] Profile each platform implementation
   [ ] Optimize hot paths
   [ ] Tune retry logic and backoff
   [ ] Minimize allocations
   ```

2. **Documentation**
   ```
   [ ] API documentation for all public items
   [ ] Platform-specific behavior notes
   [ ] Migration guide from current version
   [ ] Performance characteristics documentation
   [ ] Examples for each platform
   ```

3. **Final Testing**
   ```
   [ ] Run full test suite on all platforms
   [ ] Long-running stress tests
   [ ] Memory leak detection
   [ ] Fuzzing (if applicable)
   ```

4. **Benchmarking**
   ```
   [ ] Complete benchmark suite
   [ ] Cross-platform comparison
   [ ] Document results
   [ ] Performance regression tests
   ```

---

## Platform-Specific Details

### Linux (io_uring futex)

#### Requirements

- **Kernel:** Linux 6.7+ for futex operations
- **Fallback:** Use generic implementation on older kernels
- **Detection:** Check kernel version at runtime

#### Implementation Strategy

```rust
#[cfg(target_os = "linux")]
pub fn create_waiter_queue() -> Box<dyn WaiterQueueTrait> {
    // Try to get io_uring handle from compio runtime
    if let Some(uring) = try_get_uring_handle() {
        // Check if futex operations are supported
        if uring.supports_futex() {
            return Box::new(LinuxWaiterQueue::new(uring));
        }
    }
    
    // Fallback to generic
    Box::new(GenericWaiterQueue::new())
}
```

#### Testing Considerations

- Test on multiple kernel versions (6.6, 6.7, 6.8+)
- Test fallback behavior
- Test io_uring unavailable scenario
- Verify no memory leaks in submission queue

#### Known Issues

- Futex operations require Linux 6.7+
- Need to handle submission queue full
- Need to coordinate with compio runtime's io_uring instance

---

### Windows (IOCP)

#### Requirements

- **OS:** Windows 8+ for NtAssociateWaitCompletionPacket
- **Fallback:** Use generic implementation on Windows 7
- **Detection:** Check Windows version at runtime

#### Implementation Strategy

```rust
#[cfg(windows)]
pub fn create_waiter_queue() -> Box<dyn WaiterQueueTrait> {
    // Try to get IOCP handle from compio runtime
    if let Some(iocp) = try_get_iocp_handle() {
        // Check if NtAssociateWaitCompletionPacket is available
        if has_nt_associate_wait_completion() {
            return Box::new(WindowsWaiterQueue::new(iocp));
        }
    }
    
    // Fallback to generic
    Box::new(GenericWaiterQueue::new())
}
```

#### Testing Considerations

- Test on Windows 10, 11, Server 2019+
- Test fallback behavior on Windows 7
- Test IOCP unavailable scenario
- Verify proper cleanup of events

#### Known Issues

- NtAssociateWaitCompletionPacket is undocumented
- Need to handle IOCP full scenario
- Need to coordinate with compio runtime's IOCP instance

---

### Generic (All Others)

#### Requirements

- **Platforms:** macOS, BSD, embedded, any other
- **Dependencies:** crossbeam-queue or parking_lot

#### Implementation Strategy

Two options:

**Option A: parking_lot + AtomicWaker**
```rust
pub struct GenericWaiterQueue {
    mode: AtomicU8,
    single: AtomicWaker,
    multi: parking_lot::Mutex<VecDeque<Waker>>,
}
```

**Option B: crossbeam-queue (pure lock-free)**
```rust
pub struct GenericWaiterQueue {
    waiters: crossbeam_queue::SegQueue<Waker>,
}
```

#### Testing Considerations

- Test on macOS (CI)
- Test on BSD (if possible)
- Verify lock-free behavior (loom testing)
- Stress test under high contention

---

## Testing Strategy

### Test Categories

1. **Unit Tests** - Test individual components
2. **Integration Tests** - Test interaction with compio runtime
3. **Platform-Specific Tests** - Test platform-specific behavior
4. **Cross-Platform Tests** - Same tests run on all platforms
5. **Stress Tests** - High load, edge cases
6. **Benchmarks** - Performance measurements

### Test Matrix

| Test Type | Linux | Windows | macOS | Coverage |
|-----------|-------|---------|-------|----------|
| Unit | ✅ | ✅ | ✅ | 90%+ |
| Integration | ✅ | ✅ | ✅ | 80%+ |
| Platform-Specific | ✅ io_uring | ✅ IOCP | ❌ N/A | 100% |
| Stress | ✅ | ✅ | ✅ | Manual |
| Benchmarks | ✅ | ✅ | ✅ | All cases |

### Cross-Platform Test Suite

```rust
// tests/semaphore_tests.rs
// These tests run on ALL platforms

#[compio::test]
async fn test_basic_acquire_release() {
    let sem = Semaphore::new(1);
    let permit = sem.acquire().await;
    assert_eq!(sem.available_permits(), 0);
    drop(permit);
    assert_eq!(sem.available_permits(), 1);
}

#[compio::test]
async fn test_multiple_waiters() {
    let sem = Arc::new(Semaphore::new(1));
    
    // Acquire the only permit
    let permit = sem.acquire().await;
    
    // Spawn waiters
    let mut handles = vec![];
    for i in 0..10 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            let _p = sem.acquire().await;
            i
        }));
    }
    
    // Release permit
    drop(permit);
    
    // All should complete
    for h in handles {
        h.await.unwrap();
    }
}

#[compio::test]
async fn test_try_acquire() {
    let sem = Semaphore::new(1);
    
    let p1 = sem.try_acquire();
    assert!(p1.is_some());
    
    let p2 = sem.try_acquire();
    assert!(p2.is_none());
    
    drop(p1);
    
    let p3 = sem.try_acquire();
    assert!(p3.is_some());
}

#[compio::test]
async fn test_concurrent_acquire_release() {
    let sem = Arc::new(Semaphore::new(10));
    let mut handles = vec![];
    
    for _ in 0..100 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            let _p = sem.acquire().await;
            // Hold permit briefly
            compio::runtime::yield_now().await;
        }));
    }
    
    for h in handles {
        h.await.unwrap();
    }
    
    assert_eq!(sem.available_permits(), 10);
}
```

### Linux-Specific Tests

```rust
// tests/linux_specific.rs

#![cfg(target_os = "linux")]

use compio_sync::Semaphore;

#[compio::test]
async fn test_linux_unified_event_loop() {
    // Verify that semaphore waits use io_uring
    let sem = Semaphore::new(1);
    
    // This should go through io_uring futex
    let _p = sem.acquire().await;
    
    // TODO: Add instrumentation to verify io_uring was used
}

#[compio::test]
async fn test_linux_futex_fallback() {
    // Test fallback when io_uring not available
    // (Need to mock this scenario)
}

#[test]
fn test_linux_kernel_version_detection() {
    // Test that we correctly detect kernel version
    // and choose appropriate implementation
}
```

### Windows-Specific Tests

```rust
// tests/windows_specific.rs

#![cfg(windows)]

use compio_sync::Semaphore;

#[compio::test]
async fn test_windows_iocp_integration() {
    // Verify that semaphore waits use IOCP
    let sem = Semaphore::new(1);
    
    // This should go through IOCP
    let _p = sem.acquire().await;
    
    // TODO: Add instrumentation to verify IOCP was used
}

#[compio::test]
async fn test_windows_version_fallback() {
    // Test fallback on older Windows versions
    // (Need to mock this scenario)
}
```

### Stress Tests

```rust
// tests/stress/high_contention.rs

#[compio::test]
async fn test_high_contention() {
    let sem = Arc::new(Semaphore::new(1));
    let mut handles = vec![];
    
    // 1000 tasks contending for 1 permit
    for i in 0..1000 {
        let sem = sem.clone();
        handles.push(compio::runtime::spawn(async move {
            let _p = sem.acquire().await;
            i
        }));
    }
    
    for h in handles {
        h.await.unwrap();
    }
}

#[compio::test]
async fn test_rapid_acquire_release() {
    let sem = Arc::new(Semaphore::new(10));
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];
    
    for _ in 0..100 {
        let sem = sem.clone();
        let counter = counter.clone();
        handles.push(compio::runtime::spawn(async move {
            for _ in 0..1000 {
                let _p = sem.acquire().await;
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    
    for h in handles {
        h.await.unwrap();
    }
    
    assert_eq!(counter.load(Ordering::Relaxed), 100_000);
}
```

### Cancellation Tests

```rust
// tests/stress/cancellation.rs

#[compio::test]
async fn test_future_dropped_while_waiting() {
    let sem = Arc::new(Semaphore::new(1));
    
    // Hold the permit
    let _p = sem.acquire().await;
    
    // Start acquiring but don't await
    let fut = sem.acquire();
    
    // Drop the future
    drop(fut);
    
    // Should not leak or panic
    // Semaphore should still work
    drop(_p);
    let _p2 = sem.acquire().await;
}
```

---

## CI/CD Configuration

### GitHub Actions Workflow

```yaml
# .github/workflows/ci.yml

name: CI

on:
  push:
    branches: [ main, develop ]
  pull_request:
    branches: [ main ]

env:
  RUST_BACKTRACE: 1
  CARGO_TERM_COLOR: always

jobs:
  # Test on multiple platforms
  test:
    name: Test on ${{ matrix.os }}
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
        rust: [stable, nightly]
        include:
          # Linux-specific configuration
          - os: ubuntu-latest
            test_args: --features linux-futex
            
          # Windows-specific configuration
          - os: windows-latest
            test_args: --features windows-iocp
            
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust ${{ matrix.rust }}
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.rust }}
          
      - name: Cache cargo registry
        uses: actions/cache@v3
        with:
          path: ~/.cargo/registry
          key: ${{ runner.os }}-cargo-registry-${{ hashFiles('**/Cargo.lock') }}
          
      - name: Cache cargo index
        uses: actions/cache@v3
        with:
          path: ~/.cargo/git
          key: ${{ runner.os }}-cargo-index-${{ hashFiles('**/Cargo.lock') }}
          
      - name: Cache target directory
        uses: actions/cache@v3
        with:
          path: target
          key: ${{ runner.os }}-target-${{ matrix.rust }}-${{ hashFiles('**/Cargo.lock') }}
          
      # Check kernel version on Linux
      - name: Check Linux kernel version
        if: matrix.os == 'ubuntu-latest'
        run: uname -r
        
      # Check Windows version
      - name: Check Windows version
        if: matrix.os == 'windows-latest'
        run: |
          systeminfo | findstr /B /C:"OS Name" /C:"OS Version"
          
      - name: Build
        run: cargo build --verbose ${{ matrix.test_args }}
        
      - name: Run tests
        run: cargo test --verbose ${{ matrix.test_args }}
        
      - name: Run platform-specific tests
        if: matrix.os == 'ubuntu-latest'
        run: cargo test --test linux_specific --verbose
        
      - name: Run platform-specific tests
        if: matrix.os == 'windows-latest'
        run: cargo test --test windows_specific --verbose
        
      - name: Run doc tests
        run: cargo test --doc --verbose
        
  # Stress tests (longer running)
  stress:
    name: Stress tests on ${{ matrix.os }}
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable
        
      - name: Run stress tests
        run: cargo test --release --test stress -- --test-threads=1 --nocapture
        env:
          STRESS_TEST_ITERATIONS: 100
          
  # Cross-compilation check
  cross-compile:
    name: Cross-compile check
    runs-on: ubuntu-latest
    strategy:
      matrix:
        target:
          - x86_64-unknown-linux-gnu
          - x86_64-pc-windows-gnu
          - x86_64-apple-darwin
          - aarch64-unknown-linux-gnu
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
          
      - name: Check build
        run: cargo check --target ${{ matrix.target }}
        
  # Linting
  lint:
    name: Lint and format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
          
      - name: Check formatting
        run: cargo fmt -- --check
        
      - name: Run clippy
        run: cargo clippy --all-targets --all-features -- -D warnings
        
  # Documentation
  docs:
    name: Documentation
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        
      - name: Build docs
        run: cargo doc --no-deps --all-features
        env:
          RUSTDOCFLAGS: -D warnings
```

### Benchmark Workflow

```yaml
# .github/workflows/benchmarks.yml

name: Benchmarks

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]
  schedule:
    # Run weekly
    - cron: '0 0 * * 0'

jobs:
  benchmark:
    name: Benchmark on ${{ matrix.os }}
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        
      - name: Run benchmarks
        run: cargo bench --bench semaphore --bench condvar
        
      - name: Save benchmark results
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-results-${{ matrix.os }}
          path: target/criterion/
          
      - name: Compare with baseline
        if: github.event_name == 'pull_request'
        run: |
          # Download baseline from main branch
          # Compare and comment on PR
          # (Implementation depends on specific setup)
```

### Coverage Workflow

```yaml
# .github/workflows/coverage.yml

name: Code Coverage

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

jobs:
  coverage:
    name: Code coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
          
      - name: Install cargo-llvm-cov
        run: cargo install cargo-llvm-cov
        
      - name: Generate coverage
        run: cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info
        
      - name: Upload to codecov
        uses: codecov/codecov-action@v3
        with:
          files: lcov.info
          fail_ci_if_error: true
```

---

## Benchmarking Plan

### Benchmark Suite

```rust
// benches/semaphore.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use compio_sync::Semaphore;
use std::sync::Arc;

fn bench_uncontended(c: &mut Criterion) {
    let mut group = c.benchmark_group("semaphore_uncontended");
    
    group.bench_function("try_acquire", |b| {
        let sem = Semaphore::new(100);
        b.iter(|| {
            let p = sem.try_acquire();
            black_box(p);
        });
    });
    
    group.bench_function("acquire_immediate", |b| {
        let rt = compio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| async {
            let sem = Semaphore::new(100);
            let p = sem.acquire().await;
            black_box(p);
        });
    });
    
    group.finish();
}

fn bench_contended(c: &mut Criterion) {
    let mut group = c.benchmark_group("semaphore_contended");
    
    for concurrency in [2, 4, 8, 16, 32, 64, 128].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            concurrency,
            |b, &concurrency| {
                let rt = compio::runtime::Runtime::new().unwrap();
                b.to_async(&rt).iter(|| async {
                    let sem = Arc::new(Semaphore::new(10));
                    let mut handles = vec![];
                    
                    for _ in 0..concurrency {
                        let sem = sem.clone();
                        handles.push(compio::runtime::spawn(async move {
                            let _p = sem.acquire().await;
                            black_box(42);
                        }));
                    }
                    
                    for h in handles {
                        h.await.unwrap();
                    }
                });
            },
        );
    }
    
    group.finish();
}

fn bench_acquire_release_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("semaphore_cycle");
    
    group.bench_function("single_thread", |b| {
        let rt = compio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| async {
            let sem = Semaphore::new(1);
            for _ in 0..1000 {
                let p = sem.acquire().await;
                drop(p);
            }
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_uncontended,
    bench_contended,
    bench_acquire_release_cycle
);
criterion_main!(benches);
```

### Platform Comparison

```rust
// benches/platform_comparison.rs

use criterion::{criterion_group, criterion_main, Criterion};
use compio_sync::Semaphore;
use std::sync::Arc;

fn compare_platforms(c: &mut Criterion) {
    let mut group = c.benchmark_group("platform_comparison");
    
    // This will run on each platform and we can compare results
    group.bench_function("unified_vs_dual_event_loop", |b| {
        let rt = compio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| async {
            let sem = Arc::new(Semaphore::new(1));
            
            // Mix I/O and sync operations
            // On Linux/Windows: should use unified event loop
            // On macOS: uses dual event loop
            
            let sem2 = sem.clone();
            let h = compio::runtime::spawn(async move {
                let _p = sem2.acquire().await;
                // Simulate some I/O
                compio::fs::read_to_string("/dev/null").await.ok();
            });
            
            h.await.unwrap();
        });
    });
    
    group.finish();
}

criterion_group!(benches, compare_platforms);
criterion_main!(benches);
```

### Results Collection

Create a script to collect and compare benchmark results:

```bash
# scripts/benchmark_all_platforms.sh

#!/bin/bash

echo "Running benchmarks on all platforms..."

# Run on each platform via CI or manually
# Save results to docs/benchmarks/results/

# Linux
cargo bench --bench semaphore > docs/benchmarks/results/linux.txt

# Generate comparison report
python scripts/compare_benchmarks.py \
    docs/benchmarks/results/linux.txt \
    docs/benchmarks/results/windows.txt \
    docs/benchmarks/results/macos.txt \
    > docs/benchmarks/comparison.md
```

---

## Timeline and Milestones

### Week 1-2: Phase 1 (Generic Implementation)

**Milestones:**
- [ ] Day 1-2: Refactor existing code, extract WaiterQueue interface
- [ ] Day 3-4: Implement generic WaiterQueue (parking_lot or crossbeam)
- [ ] Day 5-6: Write comprehensive tests
- [ ] Day 7-8: Benchmark and choose best implementation
- [ ] Day 9-10: Documentation and code review

**Deliverable:** Working baseline on all platforms

### Week 3-4: Phase 2 (Linux Implementation)

**Milestones:**
- [ ] Day 1-3: Research io_uring futex API, design integration
- [ ] Day 4-6: Implement Linux WaiterQueue
- [ ] Day 7-8: Integration testing with compio
- [ ] Day 9-10: Stress testing and bug fixes
- [ ] Day 11-12: Documentation and benchmarks
- [ ] Day 13-14: Code review and refinement

**Deliverable:** Linux unified event loop working

### Week 5-6: Phase 3 (Windows Implementation)

**Milestones:**
- [ ] Day 1-3: Research Windows IOCP API, design integration
- [ ] Day 4-6: Implement Windows WaiterQueue
- [ ] Day 7-8: Integration testing with compio
- [ ] Day 9-10: Stress testing and bug fixes
- [ ] Day 11-12: Documentation and benchmarks
- [ ] Day 13-14: Code review and refinement

**Deliverable:** Windows unified event loop working

### Week 7-8: Phase 4 (Polish & Release)

**Milestones:**
- [ ] Day 1-2: Performance profiling and optimization
- [ ] Day 3-4: Complete documentation
- [ ] Day 5-6: Final testing (all platforms)
- [ ] Day 7-8: Benchmark comparison and analysis
- [ ] Day 9-10: CHANGELOG, migration guide
- [ ] Day 11-12: Pre-release testing
- [ ] Day 13-14: Release preparation

**Deliverable:** Ready for release

---

## Risk Management

### Identified Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| io_uring API changes | Low | High | Version detection, fallback |
| IOCP API unavailable | Low | High | Windows version detection, fallback |
| Performance regression | Medium | High | Comprehensive benchmarks |
| Platform-specific bugs | High | Medium | Extensive CI testing |
| Integration issues with compio | Medium | High | Early integration, close collaboration |
| Kernel version compatibility | Medium | Medium | Runtime detection, fallback |

### Mitigation Strategies

1. **Fallback Mechanisms**
   - Always have generic implementation as fallback
   - Runtime detection of capabilities
   - Graceful degradation

2. **Comprehensive Testing**
   - Test on multiple OS versions
   - CI on all target platforms
   - Stress tests and edge cases

3. **Clear Documentation**
   - Document requirements clearly
   - Explain platform differences
   - Provide migration guide

4. **Performance Monitoring**
   - Baseline benchmarks before starting
   - Regular benchmark runs
   - Performance regression tests in CI

---

## Success Criteria

### Must Have (MVP)

- ✅ Generic implementation works on all platforms
- ✅ Linux implementation uses io_uring futex
- ✅ Windows implementation uses IOCP
- ✅ Fallback to generic when platform features unavailable
- ✅ All existing tests pass on all platforms
- ✅ No performance regression vs current implementation
- ✅ CI tests on Linux, Windows, macOS

### Should Have

- ✅ 20%+ performance improvement on Linux
- ✅ 20%+ performance improvement on Windows
- ✅ Comprehensive documentation
- ✅ Migration guide
- ✅ Benchmark results published

### Nice to Have

- ✅ 40%+ performance improvement under contention
- ✅ Zero-copy operations where possible
- ✅ Fuzzing tests
- ✅ loom testing for lock-free code
- ✅ Video/blog post explaining architecture

---

## Next Steps

1. **Review this plan** with team/maintainers
2. **Set up project board** with tasks from each phase
3. **Configure CI** (GitHub Actions workflows)
4. **Create baseline branch** for benchmarking
5. **Begin Phase 1** implementation

---

**Document Status:** Ready for Implementation  
**Last Updated:** 2025-10-21  
**Owner:** compio-sync team

