# Phase 2 Implementation Plan: Linux io_uring Futex

**Branch**: `phase2-linux-io-uring-futex`  
**Depends On**: Phase 1 (platform-specific architecture)  
**Timeline**: 2-3 weeks  
**Status**: 🚧 In Progress

---

## Goal

Implement unified event loop on Linux by using io_uring futex operations for synchronization, allowing both I/O and sync primitives to be handled through the same completion queue.

---

## Prerequisites Research

### 1. io_uring Futex Operations (Linux 6.7+)

**Operations**:
- `IORING_OP_FUTEX_WAIT` - Async wait on futex
- `IORING_OP_FUTEX_WAKE` - Wake waiters on futex

**Key Points**:
- Requires Linux kernel 6.7+
- Part of io_uring's extended operation set
- Allows mixing I/O and synchronization in single event loop

### 2. compio Integration

**Questions to Answer**:
- [ ] How does compio expose io_uring instance?
- [ ] Can we submit custom operations?
- [ ] How to handle completion events?
- [ ] What's the fallback mechanism if futex ops unsupported?

**Research Needed**:
```bash
# Check compio-driver API
grep -r "io_uring" compio/
grep -r "submit" compio/
grep -r "Driver" compio/
```

---

## Implementation Steps

### Step 1: Research compio-driver API (1-2 days)

**Tasks**:
- [ ] Study `compio-driver` crate documentation
- [ ] Find how to access Driver instance from runtime
- [ ] Understand submission/completion flow
- [ ] Check if futex operations are already supported
- [ ] Determine integration points

**Expected Discovery**:
```rust
// Hypothetical API we need to find
compio::runtime::current_driver()  // Get driver instance?
driver.submit_io_uring_op(...)     // Submit custom op?
driver.register_completion(...)    // Handle completion?
```

### Step 2: Kernel Version Detection (1 day)

**Tasks**:
- [ ] Implement runtime kernel version check
- [ ] Create feature detection for futex operations
- [ ] Implement graceful fallback to generic

**Code Outline**:
```rust
// src/waiter_queue/linux.rs

fn supports_io_uring_futex() -> bool {
    // Check kernel version >= 6.7
    // Check if io_uring supports futex ops
    // Return true/false
}

pub fn new() -> WaiterQueue {
    if supports_io_uring_futex() {
        WaiterQueue::IoUring(IoUringWaiterQueue::new())
    } else {
        WaiterQueue::Generic(GenericWaiterQueue::new())
    }
}
```

### Step 3: Implement IoUringWaiterQueue (1 week)

**File**: `src/waiter_queue/linux.rs`

**Structure**:
```rust
pub enum WaiterQueue {
    IoUring(IoUringWaiterQueue),
    Generic(GenericWaiterQueue),
}

struct IoUringWaiterQueue {
    driver: Arc<CompioDriver>,  // Reference to compio's driver
}
```

**Methods to Implement**:

#### 3a. `add_waiter_if`
```rust
pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool {
    // Fast path: atomic CAS check
    if condition() {
        return true;
    }
    
    // Slow path: submit IORING_OP_FUTEX_WAIT
    // This registers the wait with io_uring
    // When woken, io_uring will post completion
    // compio runtime will poll completion and wake the waker
    
    todo!("Submit futex wait to io_uring")
}
```

#### 3b. `wake_one`
```rust
pub fn wake_one(&self) {
    // Submit IORING_OP_FUTEX_WAKE with count=1
    // io_uring will wake one waiter
    
    todo!("Submit futex wake to io_uring")
}
```

#### 3c. `wake_all`
```rust
pub fn wake_all(&self) {
    // Submit IORING_OP_FUTEX_WAKE with count=INT_MAX
    // io_uring will wake all waiters
    
    todo!("Submit futex wake to io_uring")
}
```

### Step 4: Handle Atomics as Futex Addresses (2 days)

**Challenge**: Futex operations work on memory addresses. We need to coordinate with the atomic permits counter in Semaphore.

**Approach**:
```rust
// In Semaphore::acquire()
async fn acquire(&self) -> SemaphorePermit {
    loop {
        // Fast path
        if let Some(permit) = self.try_acquire() {
            return permit;
        }
        
        // Slow path: wait on the permits atomic
        self.waiters.wait_on_atomic(&self.permits, expected_value).await;
        
        // Retry after wake
    }
}
```

### Step 5: Integration Testing (3-4 days)

**Test File**: `tests/linux_specific.rs`

**Tests to Write**:
- [ ] Verify io_uring is used (instrumentation)
- [ ] Test with io_uring available
- [ ] Test fallback when io_uring unavailable
- [ ] Test on old kernel (fallback to generic)
- [ ] Test on new kernel (use futex ops)
- [ ] Stress test with high concurrency
- [ ] Verify unified event loop (mix I/O and sync)

**Example Test**:
```rust
#[cfg(target_os = "linux")]
#[compio::test]
async fn test_unified_event_loop() {
    let sem = Arc::new(Semaphore::new(1));
    
    // This should use io_uring futex (if kernel 6.7+)
    let _p = sem.acquire().await;
    
    // Mix with I/O operation
    let content = compio::fs::read_to_string("/etc/hostname").await.unwrap();
    
    // Both should go through same io_uring completion queue
    assert!(!content.is_empty());
}
```

### Step 6: Benchmarking (2 days)

**Compare**:
- Generic implementation (baseline)
- Linux io_uring futex implementation

**Metrics**:
- Uncontended latency (should be same)
- Contended latency (may be higher due to kernel round-trip)
- Throughput under load
- Event loop simplicity (unified vs dual)

**Expected Results**:
```
Uncontended: ~same (both use atomic fast path)
Contended:   ~5-10μs (kernel futex operation)
Benefit:     Unified event loop (architectural win)
```

### Step 7: Documentation (1 day)

**Update**:
- [ ] `src/waiter_queue/linux.rs` - Implementation docs
- [ ] `README.md` - Linux-specific features
- [ ] `CHANGELOG.md` - Linux io_uring support
- [ ] Kernel version requirements

---

## Potential Challenges

### Challenge 1: compio May Not Expose Low-Level io_uring

**If**: compio doesn't expose raw io_uring access

**Solution Options**:
1. **PR to compio**: Add API for custom operations
2. **Bypass**: Use io-uring crate directly (separate instance)
3. **Wait**: Stick with generic until compio adds support

**Likely**: Option 1 (contribute to compio)

### Challenge 2: Futex Operations May Not Be Available

**If**: Kernel < 6.7 or io_uring doesn't support futex

**Solution**: ✅ Already planned! Graceful fallback to generic.

```rust
pub fn new() -> WaiterQueue {
    match detect_capabilities() {
        Capabilities::IoUringFutex => WaiterQueue::IoUring(...),
        _ => WaiterQueue::Generic(...),
    }
}
```

### Challenge 3: Waker Storage with io_uring

**Problem**: io_uring tracks operations by user_data, but we need to wake specific wakers.

**Solution**: 
```rust
// Store wakers in a concurrent map
// Key: operation ID
// Value: Waker

struct IoUringWaiterQueue {
    driver: Arc<CompioDriver>,
    pending_wakers: DashMap<u64, Waker>,  // concurrent hashmap
}

// On submission:
let op_id = next_op_id();
pending_wakers.insert(op_id, waker);
submit_futex_wait_with_user_data(op_id);

// On completion:
if let Some(waker) = pending_wakers.remove(&op_id) {
    waker.wake();
}
```

---

## Success Criteria

### Must Have
- [ ] Works on Linux 6.7+
- [ ] Falls back gracefully on older kernels
- [ ] All existing tests pass
- [ ] New Linux-specific tests pass
- [ ] No performance regression vs generic
- [ ] Documentation complete

### Should Have
- [ ] Unified event loop demonstrated
- [ ] Performance improvement in some scenarios
- [ ] Benchmarks show trade-offs clearly
- [ ] Integration with compio runtime is clean

### Nice to Have
- [ ] Performance improvement across the board
- [ ] Contribution to compio for futex support
- [ ] Blog post explaining the implementation

---

## Timeline

```
Week 1:
  Day 1-2: Research compio API
  Day 3:   Kernel detection
  Day 4-5: Start implementation

Week 2:
  Day 1-3: Complete implementation
  Day 4-5: Testing

Week 3:
  Day 1-2: Benchmarking
  Day 3:   Documentation
  Day 4-5: PR review and refinement
```

---

## Next Actions

1. ✅ Create Phase 2 branch
2. 🚧 Research compio-driver API
3. ⬜ Implement kernel detection
4. ⬜ Implement IoUringWaiterQueue
5. ⬜ Write tests
6. ⬜ Benchmark
7. ⬜ Document

---

## Research Commands

```bash
# Study compio-driver
find ~/.cargo/registry -name "compio-driver*" -type d
cd <compio-driver-path>
grep -r "io_uring" .
grep -r "submit" .

# Check io-uring crate for futex support
cargo search io-uring
cargo doc --open io-uring

# Check kernel version
uname -r

# Test futex operations availability
# (will need to write small test program)
```

---

**Status**: Ready to begin research phase!

**Next Step**: Research compio-driver API to understand integration points.

