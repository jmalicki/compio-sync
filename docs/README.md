# compio-sync Documentation

Documentation for the platform-specific async synchronization primitives in compio-sync.

## Overview

compio-sync provides async synchronization primitives (Semaphore, Condvar, etc.) optimized for the compio runtime with platform-specific implementations where beneficial.

## Documentation Structure

### Research & Design

- **[research/](research/)** - Research on async synchronization approaches
  - [mutex-free-wakeup-research.md](research/mutex-free-wakeup-research.md) - Comprehensive analysis of lock-free wakeup mechanisms
  - [wakeup-approaches-comparison.md](research/wakeup-approaches-comparison.md) - Comparison of different wakeup strategies
  - [README.md](research/README.md) - Research overview

- **[design/](design/)** - Design documentation
  - [semaphore-design.md](design/semaphore-design.md) - Semaphore implementation design
  - [README.md](design/README.md) - Design overview

### Implementation Plans

- **[implementation/](implementation/)** - Implementation planning
  - [implementation-plan-lockfree.md](implementation/implementation-plan-lockfree.md) - Three-phase plan for lock-free optimizations

## Current Status

**Phase 1 Complete**: Generic implementation with lock-free single-waiter optimization
- AtomicWaker for single-waiter fast path (lock-free!)
- parking_lot::Mutex for multi-waiter path
- Atomic mode state machine

**Future Phases**:
- Phase 2: Platform-specific optimizations (io_uring for Linux, IOCP for Windows)
- Phase 3: Advanced optimizations (intrusive lists, etc.)

## Key Concepts

### Waiter Queue Architecture

The waiter queue uses a hybrid approach:
- **Empty state**: No waiters, instant operations
- **Single state**: One waiter using AtomicWaker (lock-free atomic operations)
- **Multi state**: Multiple waiters using parking_lot::Mutex + VecDeque

### Performance Characteristics

- **Single waiter** (most common): Lock-free atomic operations (~nanoseconds)
- **Multiple waiters**: Fast parking_lot mutex (2-5x faster than std::Mutex)
- **Platform-specific** (future): io_uring/IOCP for optimal OS integration

## Contributing

When adding new synchronization primitives or optimizations:
1. Review the research documentation for background
2. Follow the design patterns in existing primitives
3. Consider the three-phase approach: generic → platform-specific → advanced
4. Ensure thorough testing (unit, integration, and stress tests)

## References

The research documents include extensive references to:
- Tokio's synchronization primitives
- CrossBeam lock-free data structures
- parking_lot fast mutex implementation
- async-lock and event-listener crates
- Platform-specific APIs (io_uring, IOCP, kqueue)
