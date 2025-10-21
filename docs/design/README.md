# Design Documents

This directory contains architectural designs and specifications for compio-sync primitives.

---

## 📄 Documents

### [Semaphore Design](semaphore-design.md)
**Type**: Original Design | **Length**: ~10 pages | **Audience**: Understanding baseline

The original design document for the semaphore implementation:

**What's Inside:**
- Problem statement (unbounded concurrency issues)
- Solution (bounded concurrency with semaphore)
- Architecture diagrams
- Key design decisions
- Integration points
- Performance characteristics
- Testing strategy
- Future enhancements

**Key Concepts:**
- Permit acquisition scope (per directory entry)
- Lock-free fast path
- FIFO waiter queue
- RAII permits
- CLI integration (`--max-files-in-flight`)

**Use Cases:**
- Bounding concurrency
- Resource protection
- Backpressure
- Rate limiting

**Read this to**: Understand why we built the original semaphore.

---

## 🏛️ Architecture Evolution

### Original Architecture (v0.0.1)

```rust
pub struct Semaphore {
    permits: AtomicUsize,            // Lock-free fast path
    waiters: Mutex<VecDeque<Waker>>, // Simple mutex for waiters
}
```

**Characteristics:**
- Simple and correct
- Uses std::sync::Mutex
- Fast uncontended path
- Safe for async (no `.await` in critical section)

**Limitations:**
- Not truly lock-free
- std::sync::Mutex slower than alternatives
- Single implementation for all platforms

### Phase 1 Architecture (Current)

```rust
// Platform-specific via #[cfg]
pub use platform::WaiterQueue;

// Generic implementation
pub struct WaiterQueue {
    mode: AtomicU8,                  // EMPTY/SINGLE/MULTI
    single: Mutex<Option<Waker>>,    // Single-waiter fast path
    multi: Mutex<VecDeque<Waker>>,   // Multi-waiter slow path
}
```

**Improvements:**
- ✅ Platform-specific modules
- ✅ Optimized single-waiter path
- ✅ parking_lot (2-3x faster)
- ✅ Ready for io_uring/IOCP

### Future Architecture (Phase 2 & 3)

**Linux:**
```rust
pub struct WaiterQueue {
    driver: Arc<CompioDriver>,  // Uses io_uring futex
}
```

**Windows:**
```rust
pub struct WaiterQueue {
    iocp: Arc<IocpHandle>,  // Uses IOCP events
}
```

**Generic:**
```rust
pub struct WaiterQueue {
    waiters: SegQueue<Waker>,  // Lock-free queue
}
```

---

## 🎯 Design Principles

### 1. Transparent Abstraction

**Principle**: Platform details should be invisible to users.

```rust
// User code - same everywhere
let sem = Semaphore::new(100);
let permit = sem.acquire().await;
```

Implementation varies by platform, but API is identical.

### 2. Fast Path Optimization

**Principle**: Optimize for the common case (uncontended).

95%+ of operations are uncontended:
- Use atomic CAS (nanosecond-scale)
- Avoid mutex when possible
- Only pay cost when actually needed

### 3. Graceful Degradation

**Principle**: Always have a fallback.

```rust
pub fn new() -> WaiterQueue {
    if supports_platform_specific() {
        WaiterQueue::Optimized(...)
    } else {
        WaiterQueue::Generic(...)
    }
}
```

### 4. Zero-Cost Abstraction

**Principle**: No overhead for abstraction itself.

- Platform selection at compile time (`#[cfg]`)
- No runtime dispatch or trait objects (for fast path)
- Monomorphization optimizes away overhead

### 5. Safety First

**Principle**: Use safe Rust where possible.

- Current implementation: 100% safe Rust
- Phase 1-2: Safe Rust with well-tested libraries
- Phase 3: Unsafe only if benchmarks justify it

---

## 📐 Design Patterns Used

### 1. Atomic Check-and-Add

**Problem**: Prevent lost wakeups in async code

**Pattern**:
```rust
fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool {
    lock.acquire();
    if condition() {  // ← Check inside lock
        lock.release();
        return true;
    }
    queue.push(waker);  // ← Add inside lock
    lock.release();
    false
}
```

**Prevents**:
```
T1: Waiter checks condition → false
T2: Notifier sets condition → true
T3: Notifier wakes queue → (empty)
T4: Waiter adds to queue → LOST WAKEUP!
```

### 2. Try-Register-Retry (Lock-Free Alternative)

**Pattern** (for crossbeam-queue):
```rust
// First try
if condition() { return Ready; }

// Register
queue.push(waker);

// Try again (critical!)
if condition() { return Ready; }  // Spurious wakeup OK

Poll::Pending
```

### 3. Mode State Machine

**Pattern**:
```rust
EMPTY → SINGLE → MULTI
  ↑       ↓        ↓
  └───────┴────────┘
```

**Benefits**:
- Single waiter uses AtomicWaker (no mutex)
- Multi-waiter uses mutex (only when needed)
- Transitions are atomic

### 4. Lock-Then-Wake

**Pattern**:
```rust
let waker = {
    let mut queue = lock.acquire();
    queue.pop()
};  // ← Lock released

// Wake outside critical section
waker.wake();  // ← Can call arbitrary code safely
```

**Why**: `wake()` can run arbitrary code. Never hold lock during `wake()`.

---

## 🔍 Key Design Decisions

### Decision 1: Three-Tier Platform Strategy

**Question**: One implementation or platform-specific?

**Answer**: Three-tier (Linux/Windows/Generic)

**Rationale**:
- Linux and Windows both have unified event capabilities
- Other platforms benefit from fast userspace fallback
- No compromises needed
- Standard practice (Tokio, mio, parking_lot do this)

### Decision 2: parking_lot for Generic

**Question**: std::sync::Mutex, parking_lot, or crossbeam-queue?

**Answer**: parking_lot for Phase 1

**Rationale**:
- 2-3x faster than std::sync
- Simpler than crossbeam (atomic check-and-add)
- Proven and production-ready
- Can add crossbeam later if benchmarks show benefit

### Decision 3: Hybrid Single/Multi

**Question**: One path or optimize for single waiter?

**Answer**: Hybrid approach

**Rationale**:
- 90%+ of waits have only one waiter
- Atomic operations avoid mutex entirely for common case
- Fall back to mutex only when actually needed
- Small code complexity for large performance gain

### Decision 4: Transparent Abstraction

**Question**: Expose platform differences in API?

**Answer**: Hide completely via module system

**Rationale**:
- Users shouldn't care about implementation
- Same code works on all platforms
- Platform selection at compile time (zero cost)
- Can change implementation without breaking users

---

## 📊 Design Trade-Offs

| Aspect | Decision | Alternative | Why Chosen |
|--------|----------|-------------|------------|
| **Platforms** | Three-tier | Single generic | Leverage platform strengths |
| **Mutex** | parking_lot | std::sync | 2-3x faster |
| **Single waiter** | Optimized path | Always use queue | 90%+ of cases |
| **Safety** | Safe Rust | Unsafe for speed | Safety first, speed later |
| **Abstraction** | Transparent | Explicit platform | Better UX |

---

## 🚀 Future Design Considerations

### Potential Enhancements

1. **Dynamic permit adjustment**
   - Adjust permits based on system load
   - Adaptive concurrency control

2. **Per-operation-type limits**
   - Different limits for files vs directories
   - Fine-grained control

3. **Weighted permits**
   - Large operations acquire more permits
   - Better resource modeling

4. **Metrics and observability**
   - Track utilization, wait times
   - Prometheus integration?

5. **Timeout support**
   - `acquire_timeout(duration)`
   - Deadline-based waiting

---

## 📚 Related Documents

- **[Research](../research/README.md)** - Why we chose these designs
- **[Implementation](../implementation/README.md)** - How to build them
- **[Progress](../progress/PROGRESS_SUMMARY.md)** - What's done

---

**Last Updated**: 2025-10-21  
**Current Phase**: Phase 1 Complete

