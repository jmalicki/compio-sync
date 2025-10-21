# Research Documents

This directory contains comprehensive research and analysis that informed the design of platform-specific async synchronization primitives.

---

## 📄 Documents

### [Mutex-Free Wakeup Research](mutex-free-wakeup-research.md)
**Length**: ~100 pages | **Time**: 1-2 hours | **Level**: Deep technical

The complete research document analyzing all approaches to lock-free async wakeup:

**What's Inside:**
- Analysis of Tokio's intrusive linked lists
- Python asyncio's single-threaded approach
- crossbeam's lock-free queues
- event-listener and async-lock crates
- AtomicWaker pattern
- io_uring futex operations (Linux)
- Windows IOCP capabilities
- Comparative analysis and recommendations

**Key Sections:**
1. Current implementation analysis
2. Alternative approaches (6 different strategies)
3. Platform-specific capabilities
4. Trade-off analysis
5. Recommended three-tier approach
6. Implementation phases

**Read this to**: Understand *why* we chose this design.

---

### [Wakeup Approaches Comparison](wakeup-approaches-comparison.md)
**Length**: ~50 pages | **Time**: 30-60 min | **Level**: All levels

Side-by-side visual comparisons of different implementation approaches:

**What's Inside:**
- Code comparisons (current vs Phase 1 vs Phase 2/3)
- Memory ordering explanations
- Race condition prevention patterns
- Future cancellation handling
- Performance characteristics tables
- Decision matrices
- Real-world examples from Tokio and smol

**Key Sections:**
1. Current implementation (Mutex-based)
2. Phase 1 (parking_lot + AtomicWaker)
3. Phase 2 (crossbeam lock-free queue)
4. Phase 3 (Tokio-style intrusive lists)
5. Platform-specific approaches
6. Trade-off comparisons

**Read this to**: See *how* different approaches compare visually.

---

## 🎯 Key Findings

### Main Insights

1. **Fast path matters most** (95%+ of operations)
   - Uncontended operations dominate
   - Atomic CAS should be identical across all approaches
   - Optimize for the common case

2. **Platform-specific is the way**
   - No single approach is best everywhere
   - Linux: io_uring provides unified event loop
   - Windows: IOCP provides unified event loop  
   - Others: Fast userspace fallback

3. **Mutexes all use futex anyway**
   - std::sync::Mutex, parking_lot, io_uring futex - all use same kernel primitive
   - Difference is in the fast path and architecture
   - True lock-free means NO kernel calls (even in slow path)

4. **crossbeam-queue is just storage**
   - Provides lock-free queue data structure
   - Doesn't solve notification (that's Waker's job)
   - Doesn't provide unified event loop
   - Good for userspace coordination

5. **Both Linux and Windows support unified events**
   - Linux: io_uring with futex operations
   - Windows: IOCP with event association
   - This justifies platform-specific implementations

### Architecture Decision

**Three-Tier Strategy:**
```
Platform     | Implementation        | Event Loop | Status
-------------|----------------------|------------|--------
Linux        | io_uring futex       | Unified    | Phase 2
Windows      | IOCP events          | Unified    | Phase 3
Generic      | Lock-free queue      | Dual       | Phase 1 ✅
```

**Rationale**:
- Best performance on each major platform
- Unified event loops where possible
- Graceful fallback for others
- No compromises needed

---

## 📊 Approach Comparison

| Approach | Lock-Free | Complexity | Allocations | Safety | Performance |
|----------|-----------|------------|-------------|--------|-------------|
| **Current (Mutex)** | ❌ | Low | Per-waiter | Safe | Good |
| **Phase 1 (parking_lot)** | ⚠️ | Medium | Per-waiter | Safe | Very Good |
| **Phase 2 (crossbeam)** | ✅ | Medium | Per-waiter | Safe | Excellent |
| **Phase 3 (intrusive)** | ✅ | Very High | Zero | Unsafe | Best |
| **io_uring (Linux)** | ⚠️ | Medium | Per-wait | Safe | Excellent* |
| **IOCP (Windows)** | ⚠️ | Medium | Per-wait | Safe | Excellent* |

\* Excellent due to unified event loop architecture

---

## 💡 Design Principles Derived

From the research, we established these principles:

1. **Abstraction**: Platform details hidden from users
2. **Fast path first**: Optimize uncontended case
3. **Fallback always**: Must work everywhere
4. **Safety preferred**: Use safe Rust where possible
5. **Measure everything**: Benchmark before optimizing
6. **Document decisions**: Explain why, not just what

---

## 🔍 Research Methodology

1. **Literature review**: Studied Tokio, smol, async-std, Python asyncio
2. **API analysis**: Examined io_uring, IOCP, crossbeam, parking_lot
3. **Prototype testing**: Built small examples to verify assumptions
4. **Performance modeling**: Estimated costs of each approach
5. **Trade-off analysis**: Weighted complexity vs performance
6. **Decision matrices**: Systematic comparison of options

---

## 📚 References

### Rust Async Runtimes
- [Tokio](https://github.com/tokio-rs/tokio) - intrusive lists
- [smol](https://github.com/smol-rs/smol) - event-listener
- [async-std](https://async.rs) - similar to smol

### Lock-Free Libraries
- [crossbeam](https://github.com/crossbeam-rs/crossbeam) - lock-free queues
- [parking_lot](https://github.com/Amanieu/parking_lot) - fast mutex

### Platform APIs
- [io_uring](https://kernel.dk/io_uring.pdf) - Linux async I/O
- [IOCP](https://docs.microsoft.com/en-us/windows/win32/fileio/i-o-completion-ports) - Windows async I/O

### Books & Papers
- "Rust Atomics and Locks" by Mara Bos
- "The Art of Multiprocessor Programming" by Herlihy & Shavit
- "Simple, Fast, and Practical Non-Blocking Queues" (Michael & Scott, 1996)

---

## 🤔 FAQ

### Why not just use tokio::sync?

Tokio's sync primitives are tightly coupled to Tokio runtime. compio needs primitives that work with its runtime and leverage io_uring directly.

### Why not just use std::sync::Mutex forever?

std::sync::Mutex works but:
1. Not truly lock-free (uses futex in slow path)
2. Doesn't enable unified event loop
3. Slower than alternatives (parking_lot is 2-3x faster)
4. Doesn't leverage platform-specific features

### Is lock-free always faster?

No! Lock-free can be slower if:
- Contention is very low (mutex fast path is cheap)
- Retry storms occur (CAS loops waste CPU)
- Cache coherence overhead is high

Our approach: lock-free userspace for coordination, platform-specific for slow path.

### Why three implementations instead of one?

Different platforms have different strengths:
- Linux io_uring provides unified event loop
- Windows IOCP provides unified event loop
- Generic needs different approach (no unified option)

One size doesn't fit all!

### How did you verify correctness?

1. Extensive testing (24 unit tests + stress tests)
2. Studied proven implementations (Tokio, smol)
3. Memory ordering analysis
4. Race condition analysis
5. Code review

---

## 🔄 Document Updates

| Date | Update | Reason |
|------|--------|--------|
| 2025-10-21 | Initial research completed | Phase 1 foundation |
| TBD | Add Phase 2 findings | After io_uring implementation |
| TBD | Add Phase 3 findings | After IOCP implementation |

---

**Next**: Read [Implementation Plans](../implementation/README.md) to see how we're building this!

