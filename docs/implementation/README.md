# Implementation Guides

This directory contains step-by-step implementation plans and guides for building platform-specific async synchronization primitives.

---

## 📄 Documents

### [Lock-Free Implementation Plan](implementation-plan-lockfree.md)
**Length**: ~40 pages | **Type**: Roadmap | **Audience**: All implementers

High-level implementation strategy covering all three phases:

**Contents:**
- Three-phase approach overview
- Platform-specific strategies
- Testing strategy
- Benchmarking guide
- Timeline estimates
- Risk management
- Success metrics

**Phases:**
1. **Phase 1**: parking_lot + AtomicWaker (1-2 days) ✅ Done
2. **Phase 2**: crossbeam lock-free queue (1 week) or io_uring futex
3. **Phase 3**: Intrusive lists or platform-specific (3-4 weeks)

**Read this for**: Overall implementation strategy.

---

### [Detailed Implementation Plan](IMPLEMENTATION_PLAN_DETAILED.md)
**Length**: ~60 pages | **Type**: Complete guide | **Audience**: Active implementers

Complete step-by-step guide with executable instructions:

**Contents:**
- Directory structure (full layout)
- CI/CD configuration (copy-paste ready GitHub Actions)
- Testing strategy with code examples
- Benchmarking setup with Criterion
- Daily task breakdown
- Platform-specific details
- Code sketches for each component

**Includes:**
- ✅ GitHub Actions workflows (ready to use)
- ✅ Test code examples (copy-paste)
- ✅ Benchmark setup (complete)
- ✅ Risk mitigation strategies

**Read this when**: You're ready to write code.

---

### [Phase 2 Plan: Linux io_uring](PHASE2_PLAN.md)
**Length**: ~20 pages | **Type**: Specific plan | **Audience**: Linux implementers

Detailed plan for Linux io_uring futex integration:

**Contents:**
- Prerequisites and kernel requirements
- compio-driver API research tasks
- Kernel version detection strategy
- Step-by-step implementation tasks
- Integration testing approach
- Potential challenges and solutions

**Research Needed:**
- How to access compio's io_uring instance
- Whether futex operations are supported
- Completion event handling
- Fallback mechanisms

**Timeline**: 2-3 weeks

**Read this for**: Phase 2 (Linux) implementation.

---

## 🗺️ Implementation Roadmap

```
┌──────────────────────────────────────────────────┐
│ Phase 1: Foundation (COMPLETE ✅)                 │
├──────────────────────────────────────────────────┤
│ Timeline: 2 weeks                                │
│ Status: ✅ Done                                   │
│                                                  │
│ Deliverables:                                    │
│ ✅ Platform-specific module structure            │
│ ✅ Generic implementation (parking_lot)          │
│ ✅ CI/CD on Linux/Windows/macOS                  │
│ ✅ Comprehensive tests (24 passing)              │
│ ✅ Benchmark suite                               │
│ ✅ All documentation                             │
└──────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────┐
│ Phase 2: Linux io_uring (IN PROGRESS 🚧)         │
├──────────────────────────────────────────────────┤
│ Timeline: 2-3 weeks                              │
│ Status: 🚧 Planning                              │
│                                                  │
│ Tasks:                                           │
│ ⬜ Research compio-driver API                    │
│ ⬜ Kernel version detection                      │
│ ⬜ Implement IoUringWaiterQueue                  │
│ ⬜ Integration testing                           │
│ ⬜ Benchmarking                                  │
│ ⬜ Documentation                                 │
└──────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────┐
│ Phase 3: Windows IOCP (PLANNED 📅)                │
├──────────────────────────────────────────────────┤
│ Timeline: 2-3 weeks                              │
│ Status: 📅 Not started                           │
│                                                  │
│ Tasks:                                           │
│ ⬜ Research Windows IOCP API                     │
│ ⬜ OS version detection                          │
│ ⬜ Implement WindowsWaiterQueue                  │
│ ⬜ Integration testing                           │
│ ⬜ Benchmarking                                  │
│ ⬜ Documentation                                 │
└──────────────────────────────────────────────────┘
```

---

## 🎯 What Each Phase Delivers

### Phase 1 (Complete)
**Goal**: Working baseline on all platforms

**Deliverables:**
- ✅ Generic WaiterQueue using parking_lot
- ✅ 2-5x performance improvement over std::sync::Mutex
- ✅ Module structure for platform selection
- ✅ Comprehensive tests and CI

**Result**: Production-ready generic implementation.

### Phase 2 (In Progress)
**Goal**: Unified event loop on Linux

**Deliverables:**
- Linux WaiterQueue using io_uring futex operations
- Unified event loop (I/O + sync through io_uring)
- Fallback to generic on older kernels
- Benchmarks comparing unified vs dual event loops

**Result**: Architectural simplification on Linux.

### Phase 3 (Planned)
**Goal**: Unified event loop on Windows

**Deliverables:**
- Windows WaiterQueue using IOCP
- Unified event loop (I/O + sync through IOCP)
- Fallback to generic on older Windows
- Complete cross-platform optimization

**Result**: Best performance on all major platforms.

---

## 🔧 Implementation Guidelines

### Adding a New Platform Implementation

1. **Create file**: `src/waiter_queue/{platform}.rs`

2. **Implement interface**:
   ```rust
   pub struct WaiterQueue {
       // Platform-specific fields
   }
   
   impl WaiterQueue {
       pub fn new() -> Self { ... }
       pub fn add_waiter_if<F>(...) -> bool { ... }
       pub fn wake_one(&self) { ... }
       pub fn wake_all(&self) { ... }
       pub fn waiter_count(&self) -> usize { ... }
   }
   ```

3. **Add to mod.rs**:
   ```rust
   #[cfg(target_os = "your_platform")]
   mod your_platform;
   
   #[cfg(target_os = "your_platform")]
   pub use your_platform::WaiterQueue;
   ```

4. **Write tests**: `tests/{platform}_specific.rs`

5. **Update CI**: Add platform to GitHub Actions matrix

6. **Document**: Add to this README and main docs

### Code Quality Standards

- ✅ All unsafe code must be documented
- ✅ Memory ordering must be explained
- ✅ Tests must cover edge cases
- ✅ Benchmarks must compare to baseline
- ✅ Fallback mechanisms required
- ✅ Platform detection at runtime

### Testing Requirements

Every implementation must pass:
- All cross-platform tests (24 unit tests)
- Platform-specific integration tests
- Stress tests (high contention, cancellation)
- Performance regression tests

---

## 📈 Performance Targets

| Phase | Target | Metric |
|-------|--------|--------|
| Phase 1 | 2x improvement | vs std::sync::Mutex |
| Phase 2 | Unified event loop | Architectural win |
| Phase 3 | Unified event loop | Architectural win |

**Note**: Unified event loop is more about architecture than raw speed.

---

## 🧪 Testing Strategy

### Test Categories

1. **Unit tests**: Test individual components
2. **Integration tests**: Test with compio runtime
3. **Platform-specific**: Test platform features
4. **Stress tests**: High load, edge cases
5. **Benchmarks**: Performance measurements

### CI Matrix

```yaml
os: [ubuntu-latest, windows-latest, macos-latest]
rust: [stable, nightly]
```

Each platform tests its specific implementation.

---

## 📖 How to Use These Guides

### For Phase 2 (Linux)
1. Read [Phase 2 Plan](PHASE2_PLAN.md)
2. Follow step-by-step tasks
3. Refer to [Research](../research/mutex-free-wakeup-research.md) for io_uring details
4. Check [Detailed Plan](IMPLEMENTATION_PLAN_DETAILED.md) for CI/testing

### For Phase 3 (Windows)
1. Read Phase 3 Plan (TBD)
2. Follow step-by-step tasks
3. Refer to [Research](../research/mutex-free-wakeup-research.md) for IOCP details
4. Check [Detailed Plan](IMPLEMENTATION_PLAN_DETAILED.md) for CI/testing

### For Optimization
1. Run benchmarks from [Detailed Plan](IMPLEMENTATION_PLAN_DETAILED.md)
2. Profile to find bottlenecks
3. Implement optimization
4. Re-benchmark and compare

---

## 🔗 Related Documentation

- [Research](../research/README.md) - Why we chose this approach
- [Progress](../progress/PROGRESS_SUMMARY.md) - Current status
- [Design](../design/README.md) - Original architecture

---

**Status**: Phase 1 complete, Phase 2 in planning, Phase 3 queued

**Last Updated**: 2025-10-21

