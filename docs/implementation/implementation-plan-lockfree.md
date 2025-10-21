# Implementation Plan: Lock-Free Wakeup Mechanisms

## Overview

This document provides a practical, step-by-step plan to eliminate mutexes from compio-sync and achieve truly async wakeups. See [mutex-free-wakeup-research.md](./mutex-free-wakeup-research.md) for detailed research and analysis.

## Three-Phase Approach

### Phase 1: Parking Lot + AtomicWaker (Low-Hanging Fruit)
**Timeline:** 1-2 days  
**Risk:** Low  
**Benefit:** 20-30% improvement in contended scenarios

### Phase 2: CrossBeam Lock-Free Queue (True Lock-Free)
**Timeline:** 1 week  
**Risk:** Medium  
**Benefit:** 40-60% improvement in high-contention scenarios

### Phase 3: Intrusive Linked Lists (Optional, High Performance)
**Timeline:** 3-4 weeks  
**Risk:** High  
**Benefit:** Zero allocations, maximum performance

---

## Phase 1 Implementation

### Step 1.1: Add Dependencies

**Edit:** `Cargo.toml`

```toml
[dependencies]
compio = { version = "0.16", features = ["macros"] }
parking_lot = "0.12"
atomic-waker = "1.1"

[dev-dependencies]
compio = { version = "0.16", features = ["macros", "time"] }
criterion = "0.5"  # For benchmarking
```

### Step 1.2: Create Optimized WaiterQueue

**Create:** `src/waiter_queue_v2.rs`

Implement the optimized version with:
- `AtomicWaker` for single-waiter fast path (common case)
- `parking_lot::Mutex<VecDeque<Waker>>` for multi-waiter slow path
- Atomic state machine to track mode (EMPTY → SINGLE → MULTI)

**Key algorithm:**
```
add_waiter_if():
  1. Check condition (fast path exit)
  2. Try atomic transition EMPTY → SINGLE
  3. If successful, use AtomicWaker
  4. Otherwise, fall back to parking_lot::Mutex
  5. Double-check condition after registration
  
wake_one():
  1. Check mode
  2. If SINGLE, use AtomicWaker
  3. If MULTI, use mutex
  4. Update mode atomically
```

### Step 1.3: Create Benchmark Suite

**Create:** `benches/waiter_queue.rs`

Benchmark scenarios:
- Single waiter (fast path)
- Multiple waiters (slow path)
- High contention (many concurrent operations)
- Low contention
- Mixed workload

Compare:
- Current: `std::sync::Mutex`
- Phase 1: `parking_lot::Mutex + AtomicWaker`

### Step 1.4: Update WaiterQueue

**Option A: Feature flag (safer)**
```toml
[features]
default = ["parking-lot-mutex"]
parking-lot-mutex = ["parking_lot", "atomic-waker"]
std-mutex = []
```

**Option B: Direct replacement (simpler)**
Just replace the implementation in `src/waiter_queue.rs`.

### Step 1.5: Test Extensively

Run all existing tests:
```bash
cargo test --all-features
cargo test --release
```

Add new stress tests:
- High concurrency
- Future cancellation
- Spurious wakeups

### Step 1.6: Benchmark and Document

```bash
cargo bench --bench waiter_queue
```

Document results in `docs/benchmarks/phase1-results.md`:
- Baseline vs Phase 1 performance
- Memory usage
- Contention analysis

**Decision Point:** If improvement is significant (>15%), proceed to Phase 2.

---

## Phase 2 Implementation

### Step 2.1: Add CrossBeam Dependency

**Edit:** `Cargo.toml`

```toml
[dependencies]
compio = { version = "0.16", features = ["macros"] }
crossbeam-queue = "0.3"
```

### Step 2.2: Implement Lock-Free WaiterQueue

**Create:** `src/waiter_queue_lockfree.rs`

```rust
use crossbeam_queue::SegQueue;
use std::task::Waker;

pub struct WaiterQueue {
    waiters: SegQueue<Waker>,
}

impl WaiterQueue {
    pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: FnOnce() -> bool,
    {
        // Check first
        if condition() {
            return true;
        }
        
        // Add to lock-free queue
        self.waiters.push(waker);
        
        // Check again (prevent lost wakeup)
        if condition() {
            return true;  // Spurious wakeup OK
        }
        
        false
    }
    
    pub fn wake_one(&self) {
        if let Some(waker) = self.waiters.pop() {
            waker.wake();
        }
    }
    
    pub fn wake_all(&self) {
        while let Some(waker) = self.waiters.pop() {
            waker.wake();
        }
    }
}
```

**Key considerations:**
- ✅ Lock-free push/pop operations
- ⚠️ Possible spurious wakeups (document this!)
- ⚠️ Can't remove specific waiter on cancellation (acceptable)
- ✅ FIFO ordering preserved

### Step 2.3: Handle Edge Cases

**Spurious Wakeups:**
- Document that poll() must always check condition after wakeup
- This is standard practice in async Rust
- No code changes needed (already done correctly)

**Future Cancellation:**
- If Future dropped while in queue, waker becomes unused
- Next wake_one() will wake the dropped task (no-op)
- Acceptable overhead (standard async pattern)

**Memory Ordering:**
- crossbeam handles all memory ordering internally
- Uses proper Acquire/Release semantics
- No manual atomic operations needed

### Step 2.4: Update Feature Flags

```toml
[features]
default = ["lock-free"]
lock-free = ["crossbeam-queue"]
parking-lot-mutex = ["parking_lot", "atomic-waker"]
std-mutex = []
```

### Step 2.5: Comprehensive Testing

**Existing tests:** Must all pass without modification

**New tests:**
```rust
#[compio::test]
async fn test_lockfree_no_lost_wakeups() {
    // 1000 tasks waiting on semaphore with 1 permit
    // Release permits one at a time
    // Verify all 1000 tasks complete
}

#[compio::test]
async fn test_lockfree_spurious_wakeup_handling() {
    // Verify that spurious wakeups don't break correctness
    // Tasks should re-check condition and wait again if needed
}

#[compio::test]
async fn test_lockfree_high_contention() {
    // Many tasks acquiring/releasing concurrently
    // Verify count remains accurate
}

#[compio::test]
async fn test_lockfree_cancellation() {
    // Start acquire, drop Future mid-flight
    // Verify no panics, no deadlocks
}
```

**Stress testing:**
```bash
# Run tests repeatedly to catch race conditions
for i in {1..1000}; do
    cargo test --release test_lockfree_no_lost_wakeups
done
```

**Loom testing (optional but recommended):**
```toml
[dev-dependencies]
loom = "0.7"
```

### Step 2.6: Benchmark Phase 2

Compare all three implementations:
1. Baseline: std::sync::Mutex
2. Phase 1: parking_lot + AtomicWaker
3. Phase 2: crossbeam-queue

Benchmark scenarios:
- Low contention (1-2 concurrent waiters)
- Medium contention (10-50 concurrent waiters)
- High contention (100+ concurrent waiters)
- Single waiter (fast path)
- Acquire/release cycles
- Wake patterns (wake_one vs wake_all)

Document in `docs/benchmarks/phase2-results.md`.

**Decision Point:** If improvement is >30% over Phase 1, make it the default.

---

## Phase 3 Implementation (Optional)

### When to Consider Phase 3

Only pursue if ALL of the following are true:
1. ✅ Phase 2 benchmarks show it's still a bottleneck
2. ✅ Profiling shows waiter queue operations in hot path
3. ✅ Target use case has extremely high contention (1000+ concurrent waiters)
4. ✅ Team has bandwidth for complex unsafe code maintenance
5. ✅ Performance gain justifies complexity (measured >2x improvement)

### Phase 3 Overview

Implement Tokio-style intrusive linked lists:
- Waiter nodes on stack (zero allocation)
- Atomic pointer manipulation
- Complex unsafe code (~1000 lines)
- Requires Pin, careful lifetime management
- ABA problem handling

**Refer to Tokio source code:**
- `tokio/src/sync/notify.rs`
- `tokio/src/sync/batch_semaphore.rs`
- `tokio/src/loom/std/unsafe_cell.rs`

**Estimated timeline:** 3-4 weeks + 2 weeks testing

---

## Testing Strategy

### Functional Correctness

All existing tests must pass:
```bash
cargo test --all-features
cargo test --no-default-features
cargo test --release
```

### Concurrency Testing

Use `loom` for model checking (optional but recommended):
```rust
#[cfg(loom)]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;
    
    #[test]
    fn test_concurrent_acquire_release() {
        loom::model(|| {
            let sem = Arc::new(Semaphore::new(1));
            
            let threads: Vec<_> = (0..2).map(|_| {
                let sem = sem.clone();
                thread::spawn(move || {
                    // Concurrent acquire/release
                })
            }).collect();
            
            for t in threads {
                t.join().unwrap();
            }
        });
    }
}
```

### Stress Testing

```bash
# Long-running stress test
cargo test --release stress_test_high_contention -- --nocapture --test-threads=1

# Run in loop to catch rare races
./scripts/stress_test.sh 1000
```

### Performance Regression Testing

```bash
# Before changes
cargo bench --bench waiter_queue -- --save-baseline before

# After changes
cargo bench --bench waiter_queue -- --baseline before
```

---

## Rollout Strategy

### Stage 1: Development

1. Implement in feature branch
2. Code review
3. Pass all tests
4. Benchmark and document

### Stage 2: Canary Testing

1. Merge to main with feature flag OFF by default
2. Users can opt-in with `features = ["lock-free"]`
3. Gather feedback
4. Monitor for issues

### Stage 3: Default Enable

1. After 1-2 months of canary testing
2. Make lock-free the default
3. Keep old implementation as fallback
4. Document migration guide

### Stage 4: Cleanup

1. After 6-12 months with no issues
2. Remove old implementations
3. Remove feature flags
4. Simplify codebase

---

## Benchmarking Guide

### Setup

```toml
[dev-dependencies]
criterion = "0.5"

[[bench]]
name = "waiter_queue"
harness = false
```

### Benchmark Scenarios

**benches/waiter_queue.rs:**

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use compio_sync::Semaphore;
use std::sync::Arc;

fn bench_single_waiter(c: &mut Criterion) {
    c.bench_function("single_waiter_acquire_release", |b| {
        let rt = compio::runtime::Runtime::new().unwrap();
        b.to_async(rt).iter(|| async {
            let sem = Semaphore::new(1);
            let _permit = sem.acquire().await;
            // Auto-release
        });
    });
}

fn bench_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("contention");
    
    for concurrency in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            concurrency,
            |b, &concurrency| {
                let rt = compio::runtime::Runtime::new().unwrap();
                b.to_async(rt).iter(|| async {
                    let sem = Arc::new(Semaphore::new(10));
                    let mut handles = vec![];
                    
                    for _ in 0..concurrency {
                        let sem = sem.clone();
                        handles.push(compio::runtime::spawn(async move {
                            let _permit = sem.acquire().await;
                            // Do minimal work
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

criterion_group!(benches, bench_single_waiter, bench_contention);
criterion_main!(benches);
```

### Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench waiter_queue

# Compare with baseline
cargo bench -- --save-baseline before
# ... make changes ...
cargo bench -- --baseline before

# Generate HTML report
cargo bench -- --output-format bencher | tee output.txt
```

---

## Documentation Updates

### README.md

Add section on performance characteristics:
```markdown
## Performance

compio-sync uses lock-free data structures for optimal performance:
- Zero allocations in single-waiter fast path
- Lock-free queue operations for multiple waiters
- O(1) acquire and release operations
- Minimal contention under high concurrency

Benchmarks show X% improvement over mutex-based implementations
under high contention. See [benchmarks](./docs/benchmarks/) for details.
```

### API Documentation

Add notes about spurious wakeups:
```rust
/// # Spurious Wakeups
///
/// This implementation uses lock-free algorithms which may result in
/// spurious wakeups. This is standard practice in async Rust and does
/// not affect correctness. Your code should always re-check the condition
/// after being woken, which is the standard pattern for `.await`.
```

### CHANGELOG.md

```markdown
## [Unreleased]

### Changed
- **BREAKING**: Replaced mutex-based waiter queue with lock-free implementation
  using crossbeam-queue for improved performance under high contention.
- Improved performance by 40-60% in high-contention scenarios
- Reduced latency for single-waiter fast path

### Added
- Feature flags to select between implementations:
  - `lock-free` (default): CrossBeam-based lock-free implementation
  - `parking-lot-mutex`: Optimized mutex with AtomicWaker fast path
  - `std-mutex`: Original std::sync::Mutex implementation

### Notes
- Spurious wakeups are now possible (standard in async Rust)
- All existing tests pass without modification
- See docs/benchmarks/ for performance comparisons
```

---

## Success Metrics

### Phase 1 Success Criteria

- ✅ All existing tests pass
- ✅ At least 15% improvement in contended scenarios
- ✅ No regression in low-contention scenarios
- ✅ Code review approved
- ✅ Documentation updated

### Phase 2 Success Criteria

- ✅ All existing tests pass
- ✅ At least 30% improvement over Phase 1 in high contention
- ✅ Lock-free operations verified (no mutex in hot path)
- ✅ Spurious wakeup handling tested
- ✅ Future cancellation handled correctly
- ✅ Code review approved
- ✅ Documentation updated

### Overall Project Success

- ✅ No mutexes in waiter queue implementation
- ✅ Truly async wakeup mechanism
- ✅ Performance improvement demonstrated
- ✅ All tests passing
- ✅ Production-ready code quality
- ✅ Comprehensive documentation

---

## Risk Mitigation

### Risk: Performance Regression

**Mitigation:**
- Benchmark extensively before/after
- Keep old implementations available via feature flags
- Test with realistic workloads

### Risk: Subtle Concurrency Bugs

**Mitigation:**
- Extensive testing (unit, integration, stress)
- Consider loom for model checking
- Code review focused on memory ordering
- Gradual rollout with feature flags

### Risk: Increased Complexity

**Mitigation:**
- Excellent documentation and comments
- Keep implementations separate
- Start with simpler Phase 1
- Only proceed to complex phases if justified

### Risk: Spurious Wakeups Breaking User Code

**Mitigation:**
- Document clearly that spurious wakeups are standard
- Ensure all examples show correct pattern (re-check after wake)
- Verify existing code already handles this correctly (it should)

---

## Timeline

### Phase 1: 1 Week
- Day 1-2: Implementation
- Day 3-4: Testing and benchmarking
- Day 5: Code review and documentation

### Phase 2: 2-3 Weeks
- Week 1: Implementation and basic testing
- Week 2: Stress testing and benchmarking
- Week 3: Code review, documentation, cleanup

### Phase 3: 6-8 Weeks (if pursued)
- Week 1-2: Study Tokio implementation
- Week 3-5: Implementation
- Week 6-7: Testing and debugging
- Week 8: Code review and documentation

### Total: 3-12 weeks depending on phases pursued

---

## Questions for Discussion

1. **Feature flags vs direct replacement?**
   - Pro feature flags: Safer, allows A/B testing
   - Pro direct replacement: Simpler, less code to maintain

2. **When to make Phase 2 the default?**
   - After X months of testing?
   - After Y users have opted in?
   - Immediately if benchmarks are good?

3. **Do we need Phase 3 at all?**
   - What performance improvement would justify the complexity?
   - Are there target use cases that need it?

4. **Spurious wakeup documentation:**
   - How prominently to document?
   - Add examples showing correct handling?
   - Migration guide for users?

5. **Benchmark infrastructure:**
   - Run benchmarks in CI?
   - Performance regression alerts?
   - Regular benchmark reports?

---

## Resources

### Code Examples
- See `docs/mutex-free-wakeup-research.md` for detailed examples

### References
- Tokio source: https://github.com/tokio-rs/tokio
- CrossBeam: https://github.com/crossbeam-rs/crossbeam
- parking_lot: https://github.com/Amanieu/parking_lot
- async-lock: https://github.com/smol-rs/async-lock
- event-listener: https://github.com/smol-rs/event-listener

### Further Reading
- "Is Parallel Programming Hard?" by Paul McKenney
- "The Art of Multiprocessor Programming" by Herlihy and Shavit
- Tokio internals blog posts
- Rust Atomics and Locks book

---

## Next Steps

1. ✅ Review this implementation plan
2. ⬜ Approve approach and phases
3. ⬜ Create GitHub issues for each phase
4. ⬜ Set up benchmark infrastructure
5. ⬜ Begin Phase 1 implementation
6. ⬜ Regular progress reviews

---

**Document Version:** 1.0  
**Date:** 2025-10-21  
**Last Updated:** 2025-10-21

