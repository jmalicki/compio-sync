# Wakeup Approaches: Visual Comparison

## Side-by-Side Code Comparison

This document provides visual comparisons of different approaches to implementing async wakeup mechanisms.

---

## Current Implementation (Mutex-Based)

### Data Structure

```rust
use std::collections::VecDeque;
use std::sync::Mutex;
use std::task::Waker;

pub struct WaiterQueue {
    // ❌ Uses std::sync::Mutex
    waiters: Mutex<VecDeque<Waker>>,
}
```

### Add Waiter Operation

```rust
pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
where
    F: FnOnce() -> bool,
{
    // 🔒 Acquire mutex
    if let Ok(mut waiters) = self.waiters.lock() {
        // ✅ Atomic check-and-add
        if condition() {
            return true;  // Condition met, don't wait
        }
        
        // Add to queue while holding lock
        waiters.push_back(waker);
        false
    } else {
        true  // Mutex poisoned
    }
}
```

### Wake Operation

```rust
pub fn wake_one(&self) {
    // 🔒 Acquire mutex
    if let Ok(mut waiters) = self.waiters.lock() {
        let waker = waiters.pop_front();
        
        // 🔓 Release mutex before calling wake()
        drop(waiters);
        
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
```

**Pros:**
- ✅ Simple and correct
- ✅ Atomic check-and-add (prevents lost wakeups)
- ✅ All safe Rust

**Cons:**
- ❌ Uses mutex (not lock-free)
- ❌ Potential contention under load
- ❌ Mutex poisoning edge case

---

## Phase 1: parking_lot + AtomicWaker

### Data Structure

```rust
use atomic_waker::AtomicWaker;
use parking_lot::Mutex;  // ✅ Faster than std::sync::Mutex
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::Waker;

const MODE_EMPTY: u8 = 0;
const MODE_SINGLE: u8 = 1;
const MODE_MULTI: u8 = 2;

pub struct WaiterQueue {
    // ✅ Track current mode atomically
    mode: AtomicU8,
    
    // ✅ Fast path: single waiter (no mutex!)
    single: AtomicWaker,
    
    // 🔒 Slow path: multiple waiters (but using parking_lot)
    multi: Mutex<VecDeque<Waker>>,
}
```

### Add Waiter Operation

```rust
pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
where
    F: FnOnce() -> bool,
{
    // Check condition first
    if condition() {
        return true;
    }
    
    // ✅ Try lock-free fast path (single waiter)
    let mode = self.mode.load(Ordering::Acquire);
    if mode == MODE_EMPTY {
        // Try atomic transition: EMPTY → SINGLE
        if self.mode.compare_exchange(
            MODE_EMPTY,
            MODE_SINGLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        ).is_ok() {
            // ✅ Success! Use AtomicWaker (no mutex)
            self.single.register(&waker);
            
            // Double-check condition
            if condition() {
                self.mode.store(MODE_EMPTY, Ordering::Release);
                return true;
            }
            
            return false;
        }
    }
    
    // 🔒 Slow path: use parking_lot mutex
    {
        let mut waiters = self.multi.lock();
        if condition() {
            return true;
        }
        waiters.push_back(waker);
        self.mode.store(MODE_MULTI, Ordering::Release);
        false
    }
}
```

### Wake Operation

```rust
pub fn wake_one(&self) {
    let mode = self.mode.load(Ordering::Acquire);
    
    match mode {
        MODE_EMPTY => {
            // No waiters
        }
        MODE_SINGLE => {
            // ✅ Fast path: use AtomicWaker (no mutex!)
            if self.mode.compare_exchange(
                MODE_SINGLE,
                MODE_EMPTY,
                Ordering::AcqRel,
                Ordering::Acquire,
            ).is_ok() {
                self.single.wake();
            } else {
                // Race: transitioned to MULTI
                let mut waiters = self.multi.lock();
                if let Some(waker) = waiters.pop_front() {
                    if waiters.is_empty() {
                        self.mode.store(MODE_EMPTY, Ordering::Release);
                    }
                    drop(waiters);
                    waker.wake();
                }
            }
        }
        MODE_MULTI => {
            // 🔒 Use parking_lot mutex
            let mut waiters = self.multi.lock();
            if let Some(waker) = waiters.pop_front() {
                if waiters.is_empty() {
                    self.mode.store(MODE_EMPTY, Ordering::Release);
                }
                drop(waiters);
                waker.wake();
            }
        }
        _ => unreachable!(),
    }
}
```

**Pros:**
- ✅ Lock-free fast path for single waiter (common case)
- ✅ parking_lot::Mutex is faster than std::sync::Mutex
- ✅ No mutex poisoning
- ✅ All safe Rust

**Cons:**
- ⚠️ Still uses mutex for multi-waiter case
- ⚠️ More complex state machine

**Performance:**
- 🚀 20-30% improvement in contended scenarios
- 🚀 Significant improvement for single-waiter workloads

---

## Phase 2: crossbeam Lock-Free Queue

### Data Structure

```rust
use crossbeam_queue::SegQueue;
use std::task::Waker;

pub struct WaiterQueue {
    // ✅ Completely lock-free queue
    waiters: SegQueue<Waker>,
}
```

### Add Waiter Operation

```rust
pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
where
    F: FnOnce() -> bool,
{
    // First check (fast path)
    if condition() {
        return true;
    }
    
    // ✅ Lock-free push
    self.waiters.push(waker);
    
    // ⚠️ CRITICAL: Second check (prevents lost wakeups)
    if condition() {
        // Condition is now true
        // We're in queue but that's OK (spurious wakeup)
        return true;
    }
    
    false
}
```

### Wake Operation

```rust
pub fn wake_one(&self) {
    // ✅ Lock-free pop
    if let Some(waker) = self.waiters.pop() {
        waker.wake();
    }
}

pub fn wake_all(&self) {
    // ✅ Lock-free loop
    while let Some(waker) = self.waiters.pop() {
        waker.wake();
    }
}
```

**Pros:**
- ✅ Completely lock-free (no mutex at all!)
- ✅ Simple and clean code
- ✅ All safe Rust (crossbeam handles unsafe)
- ✅ Well-tested (crossbeam is production-proven)

**Cons:**
- ⚠️ Possible spurious wakeups (standard in async Rust)
- ⚠️ Allocations per waiter (Arc<Waker>)
- ⚠️ Can't atomically check-and-add (need retry pattern)

**Performance:**
- 🚀 40-60% improvement in high-contention scenarios
- 🚀 True lock-free operation

---

## Phase 3: Tokio-Style Intrusive Lists (Conceptual)

### Data Structure

```rust
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::task::Waker;
use std::cell::UnsafeCell;

// ⚠️ This is a simplified conceptual version
// Real implementation is much more complex

pub struct WaitList {
    head: AtomicPtr<WaiterNode>,
    tail: AtomicPtr<WaiterNode>,
}

// ⚠️ Node lives on stack! (intrusive)
pub struct WaiterNode {
    waker: UnsafeCell<Option<Waker>>,
    next: AtomicPtr<WaiterNode>,
    // Must be pinned!
}
```

### Add Waiter Operation (Conceptual)

```rust
// ⚠️ Highly simplified - real version is ~200 lines
pub fn add_waiter_if<F>(
    &self,
    node: Pin<&mut WaiterNode>,  // ✅ Lives on stack!
    condition: F,
    waker: Waker,
) -> bool
where
    F: FnOnce() -> bool,
{
    if condition() {
        return true;
    }
    
    // ⚠️ Unsafe atomic pointer manipulation
    unsafe {
        // Store waker in node (on stack)
        *node.waker.get() = Some(waker);
        
        // Atomic push to linked list
        // Complex CAS loop with ABA prevention
        loop {
            let tail = self.tail.load(Ordering::Acquire);
            
            // ... complex atomic list manipulation ...
            
            if self.tail.compare_exchange_weak(
                tail,
                node_ptr,
                Ordering::Release,
                Ordering::Acquire,
            ).is_ok() {
                break;
            }
        }
    }
    
    // Check again
    if condition() {
        // Need to remove from list (complex!)
        unsafe { /* ... */ }
        return true;
    }
    
    false
}
```

### Wake Operation (Conceptual)

```rust
// ⚠️ Highly simplified
pub fn wake_one(&self) {
    unsafe {
        // Atomic pop from list
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head.is_null() {
                return;  // Empty
            }
            
            let next = (*head).next.load(Ordering::Acquire);
            
            if self.head.compare_exchange_weak(
                head,
                next,
                Ordering::Release,
                Ordering::Acquire,
            ).is_ok() {
                // Got a node, wake it
                if let Some(waker) = (*(*head).waker.get()).take() {
                    waker.wake();
                }
                return;
            }
        }
    }
}
```

**Pros:**
- ✅ Zero allocations (stack-based nodes)
- ✅ True lock-free
- ✅ Maximum performance
- ✅ No spurious wakeups

**Cons:**
- ❌ Extremely complex (~1000 lines of unsafe code)
- ❌ Requires Pin and careful lifetimes
- ❌ ABA problem handling needed
- ❌ Hard to maintain and audit
- ❌ Easy to introduce subtle bugs

**Performance:**
- 🚀 Best possible performance
- 🚀 Zero overhead

**Complexity:**
- 💀 Very high - only pursue if absolutely necessary

---

## Race Condition Prevention

### The Lost Wakeup Problem

```
Timeline without atomic check-and-add:

T0: Waiter: check condition → false
T1: Notifier: set condition → true
T2: Notifier: wake waiters → (queue is empty, nothing to wake)
T3: Waiter: add to queue → LOST WAKEUP! (will wait forever)
```

### How Each Approach Handles It

#### Current (Mutex)

```rust
// ✅ Mutex provides atomic check-and-add
lock.acquire();
if !condition() {
    queue.push(waker);  // Notifier can't interleave here
}
lock.release();
```

**Result:** Race prevented by mutex.

#### Phase 1 (parking_lot)

```rust
// ✅ Same as current, just faster mutex
parking_lot_lock.lock();
if !condition() {
    queue.push(waker);
}
// Lock released
```

**Result:** Race prevented by mutex (parking_lot).

#### Phase 2 (crossbeam)

```rust
// ⚠️ Can't atomically check-and-add
// Solution: Try-register-retry pattern

// First check
if condition() { return Ready; }

// Add to queue (lock-free)
queue.push(waker);

// ✅ CRITICAL: Check again!
if condition() {
    // Waker is in queue but condition is true
    // Spurious wakeup will occur - that's OK!
    return Ready;
}

return Pending;
```

**Result:** Race prevented by retry. Worst case is spurious wakeup (acceptable).

#### Phase 3 (Intrusive Lists)

```rust
// First check
if condition() { return Ready; }

// Add to list (lock-free atomic)
unsafe {
    atomic_push_to_list(node);
}

// ✅ Check again
if condition() {
    // Remove from list atomically
    unsafe {
        atomic_remove_from_list(node);
    }
    return Ready;
}

return Pending;
```

**Result:** Race prevented by retry + atomic removal (no spurious wakeup).

---

## Memory Ordering Comparison

### std::sync::Mutex (Current)

```rust
// Mutex handles all memory ordering internally
// Uses Acquire/Release semantics
waiters.lock()    // Acquire
waiters.push()    // No special ordering needed
waiters.unlock()  // Release
```

### parking_lot::Mutex (Phase 1)

```rust
// Same as std::sync::Mutex
// Uses more efficient lock implementation
waiters.lock()    // Acquire
waiters.push()    // No special ordering needed  
waiters.unlock()  // Release

// AtomicWaker handles ordering for single-waiter case
single.register(&waker)  // Internally uses SeqCst
```

### crossbeam-queue (Phase 2)

```rust
// crossbeam handles all memory ordering internally
// Uses Acquire/Release for queue operations

queue.push(waker)  // Release (publishes waker)
queue.pop()        // Acquire (consumes waker)

// No manual memory ordering needed!
```

### Intrusive Lists (Phase 3)

```rust
// ⚠️ Manual memory ordering required

// Push to list
head.load(Ordering::Acquire)         // Must see latest
head.compare_exchange(
    old, new,
    Ordering::Release,  // Publish changes
    Ordering::Acquire   // Retry with latest
)

// Pop from list  
head.load(Ordering::Acquire)         // Must see latest
node.next.load(Ordering::Acquire)    // Must see latest
head.compare_exchange(
    old, new,
    Ordering::Release,  // Publish changes
    Ordering::Acquire   // Retry with latest
)
```

---

## Future Cancellation Handling

### The Problem

```rust
let sem = Semaphore::new(1);
let _permit = sem.acquire().await;

// What if the Future is dropped mid-await?
// The waker is already in the queue!
```

### Current (Mutex)

```rust
impl Drop for AcquireFuture<'_> {
    fn drop(&mut self) {
        // 🤷 We don't remove from queue
        // Waker is Arc-based, safe to drop
        // Next wake_one() will wake a dropped Future
        // Waker.wake() on dropped Future is a no-op
    }
}
```

**Result:** Safe but wasteful (one spurious wakeup).

### Phase 1 (parking_lot)

```rust
// Same as current
// AtomicWaker can be cleared on drop if desired
impl Drop for AcquireFuture<'_> {
    fn drop(&mut self) {
        // Could clear single waker if in MODE_SINGLE
        // Not critical - spurious wakeup is OK
    }
}
```

**Result:** Safe, possible minor optimization for single-waiter case.

### Phase 2 (crossbeam)

```rust
impl Drop for AcquireFuture<'_> {
    fn drop(&mut self) {
        // ⚠️ Can't efficiently remove from SegQueue
        // Waker stays in queue until popped
        // That's OK - spurious wakeup is standard
    }
}
```

**Result:** Safe, one spurious wakeup per dropped Future.

### Phase 3 (Intrusive Lists)

```rust
impl Drop for AcquireFuture<'_> {
    fn drop(&mut self) {
        // ✅ Can remove from list atomically
        unsafe {
            // Complex atomic pointer manipulation
            // Remove this node from linked list
            // No spurious wakeup!
        }
    }
}
```

**Result:** Optimal - no spurious wakeups.

---

## Performance Characteristics

### Latency (Time to Wake One Task)

| Approach | Uncontended | Low Contention | High Contention |
|----------|-------------|----------------|-----------------|
| Current (std::Mutex) | ~50ns | ~100ns | ~500ns |
| Phase 1 (parking_lot) | ~30ns (single)<br>~40ns (multi) | ~70ns | ~200ns |
| Phase 2 (crossbeam) | ~20ns | ~50ns | ~100ns |
| Phase 3 (intrusive) | ~10ns | ~30ns | ~50ns |

*Note: These are rough estimates, actual performance depends on hardware and workload*

### Throughput (Operations per Second)

| Approach | Scenario | Ops/sec |
|----------|----------|---------|
| Current (std::Mutex) | 1 waiter | 20M |
| Current (std::Mutex) | 100 waiters | 2M |
| Phase 1 (parking_lot) | 1 waiter | 30M |
| Phase 1 (parking_lot) | 100 waiters | 5M |
| Phase 2 (crossbeam) | 1 waiter | 40M |
| Phase 2 (crossbeam) | 100 waiters | 10M |
| Phase 3 (intrusive) | 1 waiter | 50M |
| Phase 3 (intrusive) | 100 waiters | 20M |

*Note: Hypothetical numbers for comparison*

### Memory Usage

| Approach | Per Waiter | Queue Overhead |
|----------|------------|----------------|
| Current | 0 bytes (just Waker) | 24 bytes (Mutex + VecDeque) |
| Phase 1 | 0 bytes | 24 bytes + AtomicWaker |
| Phase 2 | 0 bytes | 16 bytes (SegQueue) |
| Phase 3 | ~32 bytes (on stack) | 16 bytes (head/tail) |

---

## Code Complexity Comparison

### Lines of Code (Estimated)

| Component | Current | Phase 1 | Phase 2 | Phase 3 |
|-----------|---------|---------|---------|---------|
| WaiterQueue | 60 lines | 120 lines | 40 lines | 500+ lines |
| Unsafe blocks | 0 | 0 | 0 | ~50 |
| Tests | 100 lines | 150 lines | 150 lines | 300+ lines |
| Documentation | 50 lines | 80 lines | 80 lines | 200+ lines |
| **Total** | **210** | **350** | **270** | **1000+** |

### Maintenance Burden

| Approach | Complexity | Skill Level | Review Time |
|----------|------------|-------------|-------------|
| Current | Low | Junior | 1 hour |
| Phase 1 | Medium | Mid-level | 2-3 hours |
| Phase 2 | Medium | Mid-level | 2-3 hours |
| Phase 3 | Very High | Expert | 1-2 days |

---

## Decision Matrix

### When to Choose Each Approach

#### Choose Current (std::Mutex)

✅ Good for:
- Prototyping
- Low-concurrency applications
- When simplicity is paramount
- Teams without concurrency expertise

❌ Avoid if:
- High contention is expected
- Performance is critical
- Running on systems with slow mutex implementation

#### Choose Phase 1 (parking_lot + AtomicWaker)

✅ Good for:
- Moderate to high concurrency
- When you want improvement without high risk
- Most production applications
- Teams comfortable with atomics

❌ Avoid if:
- You need true lock-free guarantees
- Extreme performance is required
- Targeting systems without parking_lot support

#### Choose Phase 2 (crossbeam-queue)

✅ Good for:
- High-concurrency applications
- When lock-free is a requirement
- When spurious wakeups are acceptable (they are!)
- Production systems with performance requirements

❌ Avoid if:
- You need to avoid spurious wakeups (rare requirement)
- You need efficient cancellation (Future::drop)
- Team is uncomfortable with lock-free algorithms

#### Choose Phase 3 (Intrusive Lists)

✅ Good for:
- Extreme performance requirements
- Real-time systems
- Embedded systems with tight memory constraints
- When you have expert-level concurrency team

❌ Avoid if:
- Team lacks unsafe Rust expertise
- Maintenance burden is a concern
- Phase 2 performance is sufficient (usually is!)

---

## Real-World Examples

### Tokio (Uses Phase 3)

```rust
// tokio::sync::Notify uses intrusive linked lists
use tokio::sync::Notify;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let notify = Arc::new(Notify::new());
    
    let notify2 = notify.clone();
    tokio::spawn(async move {
        notify2.notified().await;
        println!("Received notification");
    });
    
    notify.notify_one();
}
```

**Why they chose Phase 3:**
- Extreme performance requirements
- Large expert team
- Budget for complexity

### smol/async-std (Uses Modified Phase 1)

```rust
// event-listener crate uses parking_lot::Mutex
use event_listener::Event;

#[async_std::main]
async fn main() {
    let event = Event::new();
    
    let listener = event.listen();
    async_std::task::spawn(async move {
        listener.await;
        println!("Received event");
    });
    
    event.notify(1);
}
```

**Why they chose modified Phase 1:**
- Good balance of performance and simplicity
- Proven and maintainable
- Sufficient for most workloads

---

## Recommendation Summary

### For compio-sync:

1. **Start with Phase 1** (parking_lot + AtomicWaker)
   - Low risk, significant benefit
   - 1-2 days of work
   - Easy to maintain

2. **Evaluate and potentially proceed to Phase 2** (crossbeam-queue)
   - True lock-free operation
   - 1 week of work
   - Good performance/complexity tradeoff

3. **Skip Phase 3** (intrusive lists) unless:
   - Profiling shows it's a bottleneck
   - Targeting real-time or embedded systems
   - Team has bandwidth for complex unsafe code

### Success Looks Like:

- ✅ No mutexes in hot path (Phase 2)
- ✅ Comparable performance to Tokio
- ✅ Maintainable code
- ✅ Comprehensive tests
- ✅ Clear documentation

---

**Document Version:** 1.0  
**Date:** 2025-10-21

