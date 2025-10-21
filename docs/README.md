# compio-sync Documentation

Welcome to the compio-sync documentation! This directory contains research, design documents, and implementation plans for building lock-free async synchronization primitives.

## 📚 Documentation Index

### 🎯 Start Here

**[Implementation Plan: Lock-Free Wakeups](./implementation-plan-lockfree.md)**
- Quick-start guide for implementing lock-free wakeups
- Step-by-step instructions for each phase
- Testing and benchmarking strategies
- Timeline and resource estimates

### 🔬 Research & Analysis

**[Mutex-Free Wakeup Research](./mutex-free-wakeup-research.md)**
- Comprehensive research on lock-free async patterns
- Analysis of Tokio, Python asyncio, and other systems
- Detailed comparison of different approaches
- Pros, cons, and trade-offs for each method
- Academic references and prior art

**[Wakeup Approaches: Visual Comparison](./wakeup-approaches-comparison.md)**
- Side-by-side code comparisons
- Visual explanation of race conditions and prevention
- Performance characteristics table
- Memory ordering analysis
- Decision matrix for choosing approaches

### 📋 Existing Design Docs

**[Semaphore Design](./semaphore-design.md)**
- Original semaphore implementation design
- Architecture and integration points
- Testing strategy
- Performance characteristics

## 🎯 Quick Navigation by Use Case

### "I want to implement lock-free wakeups"
→ Start with **[Implementation Plan](./implementation-plan-lockfree.md)**

### "I want to understand different approaches"
→ Read **[Visual Comparison](./wakeup-approaches-comparison.md)**

### "I want deep technical analysis"
→ Read **[Research Document](./mutex-free-wakeup-research.md)**

### "I want to understand the current architecture"
→ Read **[Semaphore Design](./semaphore-design.md)**

## 🗂️ Document Relationships

```
┌─────────────────────────────────────────────────────┐
│  Start: Implementation Plan (How to do it)          │
│  - Quick start guide                                │
│  - Step-by-step instructions                        │
│  - Timeline and resources                           │
└──────────────────┬──────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────┐
│  Visual Comparison (Easy to understand)             │
│  - Side-by-side code examples                       │
│  - Performance tables                               │
│  - Decision matrix                                  │
└──────────────────┬──────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────┐
│  Research Document (Deep dive)                      │
│  - Detailed analysis of each approach               │
│  - Academic references                              │
│  - Implementation details                           │
└──────────────────┬──────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────┐
│  Semaphore Design (Current implementation)          │
│  - Existing architecture                            │
│  - Integration points                               │
└─────────────────────────────────────────────────────┘
```

## 📊 Summary of Approaches

### Current Implementation
- **Tech**: `std::sync::Mutex<VecDeque<Waker>>`
- **Status**: Production-ready
- **Performance**: Good
- **Complexity**: Low

### Phase 1: parking_lot + AtomicWaker
- **Tech**: `parking_lot::Mutex` + `AtomicWaker` for single waiter
- **Timeline**: 1-2 days
- **Performance**: +20-30%
- **Complexity**: Medium
- **Risk**: Low

### Phase 2: crossbeam Lock-Free
- **Tech**: `crossbeam-queue::SegQueue`
- **Timeline**: 1 week
- **Performance**: +40-60%
- **Complexity**: Medium
- **Risk**: Medium

### Phase 3: Intrusive Lists (Optional)
- **Tech**: Tokio-style intrusive linked lists
- **Timeline**: 3-4 weeks
- **Performance**: +80-100%
- **Complexity**: Very High
- **Risk**: High

## 🎓 Learning Path

### For Beginners
1. Read the [Visual Comparison](./wakeup-approaches-comparison.md) to understand the basics
2. Review [Semaphore Design](./semaphore-design.md) to understand current architecture
3. Look at simple code examples in Visual Comparison

### For Implementers
1. Start with [Implementation Plan](./implementation-plan-lockfree.md)
2. Review [Visual Comparison](./wakeup-approaches-comparison.md) for code patterns
3. Reference [Research Document](./mutex-free-wakeup-research.md) for details as needed

### For Researchers
1. Read [Research Document](./mutex-free-wakeup-research.md) cover to cover
2. Follow references and academic papers
3. Review [Visual Comparison](./wakeup-approaches-comparison.md) for implementation details

## 🔍 Key Concepts Explained

### What is a "truly async" wakeup?

**Current (with mutex):**
```rust
// Uses blocking mutex
lock.acquire()     // Could block thread briefly
queue.push(waker)
lock.release()
```

**Lock-free (truly async):**
```rust
// Uses atomic operations only
queue.push(waker)  // Never blocks, just retries CAS
```

### Why eliminate mutexes?

1. **Performance**: Lock-free operations are faster under contention
2. **Latency**: No blocking, even briefly
3. **Scalability**: Better performance with many cores
4. **Correctness**: No deadlock risk, no priority inversion

### What are spurious wakeups?

A spurious wakeup is when a task is woken but the condition it was waiting for is still false:

```rust
// Task wakes up
let permit = sem.acquire().await;
// But semaphore might have no permits!
// Task will just wait again (handled automatically by poll())
```

**Important**: This is standard in async Rust and handled correctly by all async runtimes.

### What are intrusive linked lists?

In an intrusive list, the list node lives inside the object itself (or on its stack frame):

**Normal list:**
```rust
struct Node<T> {
    data: T,
    next: Option<Box<Node<T>>>,
}
// Node allocated on heap
```

**Intrusive list:**
```rust
struct Waiter {
    next: AtomicPtr<Waiter>,
    waker: Waker,
    // ... other fields
}
// Waiter lives on Future's stack frame!
```

**Benefits**: Zero allocations, faster
**Cost**: Complex unsafe code, requires Pin

## 🧪 Testing Strategy

All approaches must pass:

### Functional Tests
- ✅ Basic acquire/release
- ✅ Multiple waiters
- ✅ Future cancellation
- ✅ High concurrency

### Concurrency Tests
- ✅ No lost wakeups
- ✅ No deadlocks
- ✅ Correct under contention

### Performance Tests
- ✅ Benchmark vs baseline
- ✅ Low/medium/high contention
- ✅ Single vs multiple waiters

See [Implementation Plan](./implementation-plan-lockfree.md#testing-strategy) for details.

## 📈 Performance Expectations

### Phase 1 (parking_lot + AtomicWaker)
- **Single waiter**: 30-50% faster
- **Multiple waiters**: 20-30% faster
- **High contention**: 25-35% faster

### Phase 2 (crossbeam)
- **Single waiter**: 40-60% faster
- **Multiple waiters**: 40-60% faster
- **High contention**: 50-70% faster

### Phase 3 (intrusive lists)
- **Single waiter**: 80-100% faster
- **Multiple waiters**: 80-100% faster
- **High contention**: 100-150% faster

*Note: Percentages are relative to current std::sync::Mutex implementation*

## 🚦 Current Status

- ✅ Research complete
- ✅ Documentation written
- ⬜ Phase 1 implementation
- ⬜ Phase 1 benchmarks
- ⬜ Phase 2 implementation
- ⬜ Phase 2 benchmarks
- ⬜ Phase 3 evaluation

## 🤝 Contributing

When adding new documentation:

1. **Update this README** with links to your document
2. **Follow the structure**:
   - Problem statement
   - Solution approaches
   - Code examples
   - Trade-offs
   - Recommendations
3. **Include code examples** where appropriate
4. **Link to related documents**
5. **Update the "Document Relationships" diagram**

## 📝 Document Templates

### For New Design Documents

```markdown
# [Feature Name]

## Problem Statement
What problem are we solving?

## Background
What context is needed?

## Proposed Solution
How do we solve it?

## Alternatives Considered
What else did we think about?

## Trade-offs
Pros and cons of each approach

## Implementation Plan
Step-by-step guide

## Testing Strategy
How do we verify correctness?

## Performance Impact
Expected performance changes

## References
Links to prior art, papers, etc.
```

## 🔗 External Resources

### Rust Async Runtimes
- [Tokio Documentation](https://tokio.rs)
- [Tokio Source Code](https://github.com/tokio-rs/tokio)
- [smol Runtime](https://github.com/smol-rs/smol)
- [async-std](https://async.rs)

### Lock-Free Data Structures
- [crossbeam](https://github.com/crossbeam-rs/crossbeam)
- [parking_lot](https://github.com/Amanieu/parking_lot)
- [event-listener](https://github.com/smol-rs/event-listener)

### Learning Resources
- [Rust Atomics and Locks Book](https://marabos.nl/atomics/)
- [The Art of Multiprocessor Programming](https://www.amazon.com/Art-Multiprocessor-Programming-Maurice-Herlihy/dp/0123973376)
- [Is Parallel Programming Hard?](https://www.kernel.org/pub/linux/kernel/people/paulmck/perfbook/perfbook.html)

### Academic Papers
- "Simple, Fast, and Practical Non-Blocking Queues" (Michael & Scott, 1996)
- "Practical lock-freedom" (Keir Fraser, 2004)
- "Hazard Pointers: Safe Memory Reclamation" (Maged Michael, 2004)

## 📞 Questions?

- **General questions**: Check the [Visual Comparison](./wakeup-approaches-comparison.md)
- **Implementation questions**: See [Implementation Plan](./implementation-plan-lockfree.md)
- **Deep technical questions**: Read [Research Document](./mutex-free-wakeup-research.md)
- **Current architecture**: Review [Semaphore Design](./semaphore-design.md)

## 🎯 Recommended Reading Order

### Quick Overview (30 minutes)
1. This README
2. Visual Comparison (code examples section)
3. Implementation Plan (Phase 1 section)

### Implementation Ready (2 hours)
1. This README
2. Visual Comparison (full)
3. Implementation Plan (full)
4. Semaphore Design (current architecture)

### Complete Understanding (4+ hours)
1. This README
2. Semaphore Design
3. Visual Comparison
4. Research Document
5. External resources and papers

---

**Last Updated**: 2025-10-21  
**Document Version**: 1.0  
**Maintained By**: compio-sync project team

