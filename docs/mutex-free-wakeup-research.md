# Research: Mutex-Free Async Wakeup Mechanisms

## Executive Summary

This document analyzes approaches to eliminate `Mutex<VecDeque<Waker>>` from our synchronization primitives and achieve truly lock-free async wakeups. We examine implementations from Tokio, Python asyncio, and other Rust async ecosystems to design a mutex-free solution for compio-sync.

## Current Implementation Analysis

### What We Have Now

Our current implementation (`waiter_queue.rs`) uses:

```rust
pub struct WaiterQueue {
    waiters: Mutex<VecDeque<Waker>>,
}
```

**Pattern:**
1. **Check-and-add**: Lock mutex → check condition → add waker → unlock
2. **Wake**: Lock mutex → pop waker(s) → unlock → call waker.wake()

**Justification (from current code):**
- "Safe for async" because lock held for nanoseconds (no I/O, no `.await`)
- Futex-based (~2-3 cycles when uncontended)
- Solves fundamental problem: atomically checking state AND modifying queue

**Problem:**
- Still uses a mutex (even if fast)
- Can cause contention under high load
- Not truly lock-free
- Mutex poisoning edge case
- Potential priority inversion

## Research: Alternative Approaches

### 1. Tokio's Approach: Intrusive Linked Lists

#### Implementation Details

**Key Insight**: Tokio uses intrusive linked lists for lock-free synchronization primitives like `Notify` and `Semaphore`.

**Structure:**
```rust
// Simplified concept (actual Tokio code is more complex)
struct Waiter {
    waker: UnsafeCell<Option<Waker>>,
    next: AtomicPtr<Waiter>,
    // Lives on the stack of the waiting task
}

struct WaitList {
    head: AtomicPtr<Waiter>,
    tail: AtomicPtr<Waiter>,
}
```

**Key Features:**
1. **Stack-allocated nodes**: Each waiter node lives on the stack frame of the Future
2. **Atomic pointer manipulation**: Add/remove nodes using compare-and-swap
3. **Zero allocation**: No heap allocation for waiter queue
4. **Lock-free**: Uses atomic operations exclusively
5. **Intrusive**: The list structure is embedded in the Future itself

**Challenges:**
- **Complex unsafe code**: ~500-1000 lines of unsafe pointer manipulation
- **ABA problem**: Need tagged pointers or epoch-based reclamation
- **Memory ordering**: Careful use of Acquire/Release/SeqCst
- **Cancellation safety**: Handle Future drop while in queue
- **Self-referential**: Pinning required (waiter points to itself)

**Example Pattern from Tokio's Notify:**
```rust
// Conceptual, not actual code
impl Future for Notified<'_> {
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // Try fast path
        if self.notify.try_receive() {
            return Poll::Ready(());
        }
        
        // Register in intrusive list (pinned to stack)
        let waiter = Waiter {
            waker: cx.waker().clone(),
            next: AtomicPtr::new(null_mut()),
        };
        
        // Atomic push to list
        self.notify.push_waiter(&waiter);
        
        // Check again (critical for race prevention)
        if self.notify.try_receive() {
            // Remove from list
            self.notify.remove_waiter(&waiter);
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
```

**Pros:**
- ✅ Zero allocation
- ✅ True lock-free operation
- ✅ Excellent performance under high contention
- ✅ No mutex overhead

**Cons:**
- ❌ Complex unsafe code
- ❌ Requires Pin and careful lifetime management
- ❌ ABA problem handling
- ❌ Difficult to implement correctly
- ❌ Hard to maintain and audit

### 2. event-listener Crate (used by smol/async-std)

#### Implementation Details

The `event-listener` crate provides a simpler approach while still avoiding mutexes for the common case.

**Key Design:**
```rust
// Simplified concept
pub struct Event {
    // Fast path: atomic notification count
    notified: AtomicUsize,
    // Slow path: shard multiple smaller locks
    list: Mutex<LinkedList<Waker>>,  // Actually uses parking_lot::Mutex
}
```

**Features:**
1. **Parking_lot Mutex**: Uses parking_lot which is more efficient than std::sync::Mutex
2. **Fast path optimization**: Atomic counter for notification
3. **Listener struct**: Each listener has own ID and state
4. **Two-phase notification**: Notify → mark listeners → wake them

**Pattern:**
```rust
let event = Event::new();

// Listener side
let listener = event.listen();
listener.await;

// Notifier side
event.notify(1);  // Wake 1
event.notify(usize::MAX);  // Wake all
```

**Pros:**
- ✅ Much simpler than intrusive lists
- ✅ Uses parking_lot (faster mutex)
- ✅ Good performance
- ✅ Easier to understand and maintain
- ✅ Production-tested (used by smol, async-std)

**Cons:**
- ⚠️ Still uses a mutex (though parking_lot)
- ⚠️ Allocations for listener objects
- ⚠️ Not truly lock-free

### 3. async-lock Crate

#### Implementation Details

The `async-lock` crate provides async versions of synchronization primitives.

**Uses event-listener internally:**
```rust
pub struct Mutex<T> {
    state: AtomicUsize,
    lock_ops: Event,
    data: UnsafeCell<T>,
}
```

**Key insight**: Built on top of `event-listener`, which uses parking_lot::Mutex internally.

**Pros:**
- ✅ Well-tested, production-ready
- ✅ Provides full suite of primitives (Mutex, RwLock, Semaphore, etc.)
- ✅ Faster than std::sync with parking_lot

**Cons:**
- ⚠️ Still has mutex at the bottom layer
- ⚠️ Not applicable as we want to eliminate mutexes

### 4. Python asyncio Approach

#### Implementation Details

Python's asyncio takes a different approach entirely:

**Key Design:**
- **Single-threaded event loop**: No thread synchronization needed
- **Task queue in event loop**: Wakers are just task references
- **No mutex needed**: Because event loop is single-threaded
- **Deque operations**: Use collections.deque (not thread-safe, but doesn't need to be)

**Synchronization Primitives:**
```python
class Lock:
    def __init__(self):
        self._waiters = collections.deque()  # No mutex needed!
        self._locked = False
    
    async def acquire(self):
        if not self._locked:
            self._locked = True
            return
        
        fut = asyncio.get_event_loop().create_future()
        self._waiters.append(fut)
        await fut
```

**Key Insight:**
- Mutexes not needed because event loop is single-threaded
- All waker registration happens in event loop thread
- No data races possible

**Applicability to Rust:**
- ❌ Not applicable: Rust async is multi-threaded
- ❌ compio can use multiple threads
- ✅ But inspires idea: what if we could guarantee single-threaded access?

### 5. Atomic Waker Pattern

#### Implementation Details

The `AtomicWaker` pattern is used for single-waiter scenarios:

```rust
pub struct AtomicWaker {
    waker: AtomicUsize,  // Pointer to waker
}

impl AtomicWaker {
    pub fn register(&self, waker: &Waker) {
        // Atomic swap of waker pointer
        // Only supports ONE waiter
    }
    
    pub fn wake(&self) {
        // Atomic load + wake
    }
}
```

**Used in:**
- futures-util
- tokio (for some internal primitives)
- Embassy (embedded async)

**Pros:**
- ✅ Truly lock-free
- ✅ Simple implementation
- ✅ No unsafe code needed (can use Arc<Waker>)
- ✅ Very fast

**Cons:**
- ❌ Only supports single waiter
- ❌ Not applicable for queue of waiters

### 6. crossbeam-queue Approach

#### Implementation Details

The `crossbeam-queue` crate provides lock-free queues:

**Options:**
```rust
// Lock-free MPMC queue
use crossbeam_queue::SegQueue;

struct WakerQueue {
    waiters: SegQueue<Waker>,
}
```

**Features:**
- Lock-free multi-producer, multi-consumer
- Based on Michael-Scott queue algorithm
- Uses atomic pointer operations
- No unsafe in user code

#### Important: crossbeam-queue is JUST a Queue

**Key Insight**: `crossbeam-queue` itself doesn't have a "notify" or "wake" mechanism. It's just a lock-free data structure for storing items.

**How notification works:**

1. **crossbeam-queue stores the Wakers**
   ```rust
   // Just a lock-free queue, no notification built-in
   let queue = SegQueue::new();
   queue.push(waker);  // Just stores it
   ```

2. **The Waker type handles notification**
   ```rust
   if let Some(waker) = queue.pop() {
       waker.wake();  // ← This notifies the async runtime
   }
   ```

3. **The async runtime (compio) handles the actual wakeup**
   - `waker.wake()` tells the runtime "task is ready"
   - Runtime schedules the task to be polled again
   - No crossbeam involvement in the notification

**What crossbeam DOES provide for synchronous code:**

For blocking (non-async) use cases, crossbeam has different primitives:

- **`Parker`/`Unparker`** (from `crossbeam-utils`):
  ```rust
  use crossbeam_utils::thread::Parker;
  
  let parker = Parker::new();
  let unparker = parker.unparker().clone();
  
  // In one thread
  parker.park();  // Block until unparked
  
  // In another thread  
  unparker.unpark();  // Wake the parker
  ```
  - Uses OS primitives (futex on Linux)
  - For blocking/synchronous code only
  - Not applicable to async

- **`crossbeam-channel`**:
  - Uses `Parker` internally for blocking operations
  - For sync code, not async Wakers

**For async code (our use case):**

```rust
// crossbeam-queue: Just stores Wakers
let queue = SegQueue::new();
queue.push(waker);

// WE implement the notification by calling wake()
if let Some(waker) = queue.pop() {
    waker.wake();  // This goes to compio runtime
}

// compio runtime then:
// 1. Marks task as ready
// 2. Schedules it in its internal task queue
// 3. Polls the future when it gets CPU time
```

**The Stack:**
```
┌─────────────────────────────────────┐
│ Our Code: queue.push(waker)         │
├─────────────────────────────────────┤
│ crossbeam-queue: Lock-free storage  │  ← Just data structure
├─────────────────────────────────────┤
│ Our Code: waker.wake()              │
├─────────────────────────────────────┤
│ std::task::Waker                    │  ← Notification interface
├─────────────────────────────────────┤
│ compio Runtime                      │  ← Actual task scheduling
├─────────────────────────────────────┤
│ OS Thread Scheduler                 │
└─────────────────────────────────────┘
```

**Bottom line**: crossbeam-queue doesn't solve the notification problem - it only provides lock-free storage. The `Waker` type and the runtime handle notification.

**Challenges for our use case:**
```rust
// Problem: How do we atomically check condition AND add to queue?

// Current mutex approach (atomic):
lock();
if !condition() {
    queue.push(waker);
}
unlock();

// SegQueue approach (NOT atomic):
if !condition() {  // ← Race window here!
    queue.push(waker);  // ← Notification could be lost
}
```

**The Race:**
1. Waiter checks condition → false
2. Notifier sets condition → true
3. Notifier checks queue → empty (no one to wake)
4. Waiter pushes to queue → LOST WAKEUP!

**Solution:** Use try-register-retry pattern (similar to our current code):
```rust
// First check
if condition() { return Ready; }

// Add to queue
queue.push(waker);

// Check again (critical!)
if condition() {
    // Try to remove from queue (best effort)
    // Even if we can't remove, spurious wakeup is OK
    return Ready;
}

Poll::Pending
```

**Pros:**
- ✅ Lock-free queue operations
- ✅ No mutex
- ✅ Well-tested (crossbeam)
- ✅ Simpler than intrusive lists

**Cons:**
- ⚠️ Allocates per waiter (Arc<Waker>)
- ⚠️ Can't atomically check-and-add (need retry pattern)
- ⚠️ Possible spurious wakeups
- ⚠️ Can't remove specific waiter on Future drop

## Comparative Analysis

| Approach | Lock-Free | Complexity | Allocations | Safety | Maintenance | Performance |
|----------|-----------|------------|-------------|--------|-------------|-------------|
| Current (Mutex) | ❌ | Low | Per-waiter | Safe | Easy | Good |
| Tokio (Intrusive) | ✅ | Very High | Zero | Unsafe | Hard | Excellent |
| event-listener | ⚠️ | Medium | Per-listener | Safe | Medium | Very Good |
| crossbeam-queue | ✅ | Medium | Per-waiter | Safe | Medium | Very Good |
| AtomicWaker | ✅ | Low | One waker | Safe | Easy | Excellent |

## Recommended Approach for compio-sync

### Phase 1: Optimize Current Mutex (Short Term)

**Goal**: Get most benefit with least risk

**Actions:**
1. **Switch to parking_lot::Mutex**
   - Drop-in replacement for std::sync::Mutex
   - 2-3x faster in contention scenarios
   - Smaller memory footprint
   - No poisoning

2. **Add fast-path for single waiter**
   - Use AtomicWaker for common case (one waiter)
   - Fall back to Mutex<VecDeque> for multiple waiters

```rust
pub struct WaiterQueue {
    // Fast path: single waiter (most common case)
    single: AtomicWaker,
    // Slow path: multiple waiters
    multi: Mutex<Option<VecDeque<Waker>>>,  // parking_lot::Mutex
}
```

**Benefits:**
- ✅ Low risk (minimal code changes)
- ✅ Significant performance improvement
- ✅ Still safe Rust
- ✅ Easy to maintain
- ✅ No API changes

**Estimated effort:** 1-2 days

### Phase 2: Lock-Free with crossbeam-queue (Medium Term)

**Goal**: Eliminate mutex entirely for queue operations

**Approach:**
```rust
use crossbeam_queue::SegQueue;

pub struct WaiterQueue {
    waiters: SegQueue<Waker>,
    // Notification state (atomic)
    notified: AtomicUsize,
}

impl WaiterQueue {
    pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: FnOnce() -> bool,
    {
        // First check (fast path)
        if condition() {
            return true;
        }
        
        // Add to queue
        self.waiters.push(waker);
        
        // Second check (prevent lost wakeup)
        if condition() {
            // Spurious wakeup OK - will check condition again in poll
            return true;
        }
        
        false
    }
    
    pub fn wake_one(&self) {
        if let Some(waker) = self.waiters.pop() {
            waker.wake();
        }
    }
}
```

**Challenges to address:**
1. **Future cancellation**: If Future is dropped while in queue, waker might be invalid
   - Solution: Waker is Arc-based, safe to drop
   - Spurious wakeup is OK (standard async practice)

2. **FIFO ordering**: SegQueue maintains FIFO
   - ✅ Fair scheduling preserved

3. **Memory ordering**: crossbeam handles this
   - ✅ Proper acquire/release semantics

**Benefits:**
- ✅ Lock-free operations
- ✅ No mutex
- ✅ Safe Rust (crossbeam handles unsafe)
- ✅ Well-tested implementation
- ✅ Simpler than intrusive lists

**Drawbacks:**
- ⚠️ Possible spurious wakeups
- ⚠️ Allocations per waiter (but unavoidable without intrusive lists)

**Estimated effort:** 1 week

### Special Note: io_uring and Synchronization

Since compio is built on io_uring (on Linux), it's worth considering what io_uring itself provides:

#### io_uring's Built-in Capabilities

**1. Completion Queue Events (What compio uses)**

io_uring has two ring buffers:
- **Submission Queue (SQ)**: User submits I/O operations
- **Completion Queue (CQ)**: Kernel posts completion events

**How compio uses this:**
```rust
// Conceptual - actual compio implementation
loop {
    // Submit I/O operations to SQ
    ring.submit_and_wait()?;
    
    // Process completions from CQ
    for cqe in ring.completion() {
        // Wake the Future associated with this operation
        waker.wake();
    }
}
```

**Key point**: io_uring provides the I/O completion notification mechanism, but NOT general-purpose synchronization primitives.

**2. Futex Operations (Linux 6.7+)**

Recent kernels (6.7+) added futex operations to io_uring:
- `IORING_OP_FUTEX_WAIT` - Async wait on futex
- `IORING_OP_FUTEX_WAKE` - Wake waiters on futex

**Why this is interesting:**
```rust
// Hypothetically could use io_uring for synchronization
struct Semaphore {
    count: AtomicU32,
    // Could submit futex wait/wake ops through io_uring
}
```

**Why we DON'T use this:**

❌ **Not portable**: Only works on Linux 6.7+, compio targets multiple platforms
❌ **Wrong abstraction**: Futex ops are for blocking synchronization, not async
❌ **Complexity**: Would require kernel round-trips for what should be fast userspace operations
❌ **Not needed**: We already have Wakers from the runtime
❌ **Overhead**: Submitting to io_uring has overhead (fine for I/O, not for sync primitives)

**3. eventfd Integration**

io_uring can integrate with eventfd for notifications:
```rust
// Can register eventfd with io_uring
let eventfd = EventFd::new()?;
ring.register_eventfd(eventfd.as_raw_fd())?;

// Kernel will write to eventfd when CQ has entries
```

**How this relates to our problem:**
- compio likely uses eventfd or similar for runtime notifications
- This is at a lower level than our Semaphore/Condvar
- Our Wakers plug into compio's notification system
- io_uring → compio runtime → our Wakers

#### The Layering

```
┌──────────────────────────────────────────────┐
│ Our Sync Primitives (Semaphore/Condvar)     │
│ - User-space synchronization                 │
│ - Waker queue management ← WE OPTIMIZE THIS │
├──────────────────────────────────────────────┤
│ std::task::Waker                             │
│ - Notification interface                     │
├──────────────────────────────────────────────┤
│ compio Runtime                               │
│ - Task scheduling                            │
│ - Poll management                            │
│ - Uses io_uring for I/O                     │
├──────────────────────────────────────────────┤
│ io_uring (Linux) / IOCP (Windows) / etc     │
│ - Async I/O completion notification          │
│ - NOT for general synchronization           │
├──────────────────────────────────────────────┤
│ OS Kernel                                    │
└──────────────────────────────────────────────┘
```

#### Conclusion on io_uring

**io_uring is NOT the answer for our wakeup problem because:**

1. **Wrong layer**: io_uring is for I/O notifications, we need task synchronization
2. **We already have Wakers**: The runtime provides notification via std::task::Waker
3. **Performance**: io_uring operations have overhead (kernel submission), we need fast userspace operations
4. **Portability**: io_uring is Linux-specific, compio is cross-platform

**io_uring DOES help compio overall:**
- Efficient I/O completion notifications
- Fast async I/O operations
- But NOT for Semaphore/Condvar waker queues

**Our optimization is at the user-space level:**
- How we store Wakers (VecDeque vs lock-free queue)
- Whether we use a mutex or atomic operations
- This is independent of io_uring

#### Important Clarification: "What About Futex?"

**Great question**: Don't Mutex, parking_lot, etc. all use futex under the hood anyway?

**YES! You're absolutely right.** Let's clarify the layers:

**Current approach (std::sync::Mutex):**
```
Your code: mutex.lock()
    ↓
Userspace: Try atomic CAS first (fast path, no kernel)
    ↓ (only if contended)
Kernel: futex_wait() syscall ← ONLY when there's actual contention
    ↓
Wake: futex_wake() syscall
```

**Key insight**: Mutex uses futex, but with a **fast userspace path first**:
1. Try to acquire lock with atomic CAS (no kernel!)
2. Only call futex_wait() if lock is held by someone else
3. In uncontended case: pure userspace, ~2-3 CPU cycles

**If we used io_uring futex directly:**
```
Your code: submit futex wait to io_uring
    ↓
Userspace: Prepare SQE (submission queue entry)
    ↓
Kernel: io_uring submission (syscall or ring poll)
    ↓
Kernel: Process futex operation
    ↓
Kernel: Post completion to CQ
    ↓
Userspace: Poll completion queue
```

**Problem**: ALWAYS involves io_uring machinery, even when uncontended!

**The Real Difference Between Approaches:**

| Approach | Uncontended | Contended | Truly Lock-Free? |
|----------|-------------|-----------|------------------|
| **std::sync::Mutex** | Userspace CAS | futex syscall | ❌ No |
| **parking_lot::Mutex** | Userspace CAS (faster) | futex syscall | ❌ No |
| **io_uring futex** | io_uring overhead + futex | io_uring overhead + futex | ❌ No |
| **crossbeam-queue** | Userspace CAS only | Userspace CAS only | ✅ Yes! |
| **AtomicWaker** | Userspace CAS only | Userspace CAS only | ✅ Yes! |

**Key Distinction:**

1. **Mutex (any kind) = Lock-based**
   - Fast path: userspace atomic
   - Slow path: futex syscall
   - ✅ Fast when uncontended
   - ❌ Blocks on contention

2. **io_uring futex = Also lock-based + overhead**
   - Always goes through io_uring machinery
   - Still uses futex underneath
   - ❌ Overhead even when uncontended
   - ❌ Blocks on contention

3. **crossbeam-queue = Truly lock-free**
   - Only uses atomics (CAS loops)
   - Never calls kernel
   - Never blocks
   - ✅ Fast even under contention
   - ✅ Just retries CAS if fails

**The "Lock-Free" Goal:**

When we say "eliminate mutexes," we mean:
- ❌ No futex syscalls (even in slow path)
- ❌ No blocking on contention
- ✅ Just atomic operations that retry on failure
- ✅ Always userspace, never kernel

**Why crossbeam-queue is different:**
```rust
// crossbeam-queue: NO futex, NO kernel calls
pub fn push(&self, value: T) {
    loop {
        // Just atomic CAS, retries if fails
        // Never blocks, never goes to kernel
        if self.try_push_atomic(value) {
            return;
        }
        // Retry (maybe with backoff)
    }
}
```

Compare to:
```rust
// Mutex: Uses futex when contended
pub fn lock(&self) {
    // Try fast path (atomic)
    if self.try_lock_atomic() {
        return;
    }
    // Slow path: BLOCKS in kernel
    futex_wait();  // ← Syscall, thread sleeps
}
```

**Bottom Line:**

- **All mutexes** (std, parking_lot, io_uring futex) eventually use futex/kernel
- **True lock-free** (crossbeam, atomics) never touches kernel
- That's the fundamental difference we're aiming for!

#### Critical Question: "Do We Even Need a Waker Queue with io_uring?"

**Excellent question!** The answer depends on WHAT we're waiting for:

**Scenario 1: Waiting for I/O (what compio already does)**

```rust
// Reading a file
let buf = vec![0u8; 4096];
let n = file.read(&buf).await;  // Waiting for I/O
```

**What happens:**
1. compio submits read operation to io_uring's submission queue
2. Kernel performs the I/O asynchronously
3. Kernel posts completion to io_uring's completion queue
4. compio polls CQ, finds completion, wakes the Future

**Waker queue needed?** ❌ NO! io_uring itself tracks the operation and provides notification.

```
Task waiting for I/O
    ↓
No explicit waker queue needed
    ↓
io_uring completion queue IS the notification mechanism
```

**Scenario 2: Waiting for Synchronization (Semaphore/Condvar)**

```rust
// Waiting for semaphore permit
let permit = semaphore.acquire().await;  // Waiting for ANOTHER TASK
```

**What happens:**
1. Task checks: any permits available? No.
2. Task needs to wait for another task to release a permit
3. **This is NOT an I/O operation!**
4. There's nothing to submit to io_uring
5. We need to store the waker somewhere so we can wake it later

**Waker queue needed?** ✅ YES! There's no I/O operation, just task coordination.

```
Task waiting for permit
    ↓
Need to store waker somewhere
    ↓
When another task releases permit, need to find and wake this task
    ↓
That's what our waker queue does
```

**The Key Distinction:**

| Waiting For | Kernel Involved? | Can Use io_uring? | Need Waker Queue? |
|-------------|------------------|-------------------|-------------------|
| **I/O operation** (read, write, etc.) | ✅ Yes | ✅ Yes (io_uring tracks it) | ❌ No |
| **Synchronization** (semaphore, condvar) | ❌ No (userspace only) | ❌ No (nothing to submit) | ✅ Yes |

**Why We Can't Use io_uring for Semaphore:**

```rust
// This is NOT an I/O operation!
impl Semaphore {
    pub async fn acquire(&self) -> SemaphorePermit {
        // Option 1: Pure userspace (current approach)
        if let Some(permit) = self.try_acquire() {
            return permit;  // Fast path, no kernel
        }
        
        // No permit available, must wait
        // What do we submit to io_uring???
        // There's no file, no socket, no I/O operation!
        // Just waiting for another task to call release()
        
        // So we store the waker in a queue
        self.waiters.push(waker);
        pending
    }
    
    pub fn release(&self) {
        self.permits.fetch_add(1);
        
        // Wake a waiter - THIS is pure userspace coordination
        // No kernel involved, no I/O involved
        if let Some(waker) = self.waiters.pop() {
            waker.wake();  // Just tells runtime to schedule task
        }
    }
}
```

**Could We Use io_uring Futex Operations?**

Theoretically, yes:
```rust
// Hypothetical - NOT recommended
pub async fn acquire(&self) {
    if self.try_acquire() {
        return;
    }
    
    // Submit futex wait to io_uring
    let sqe = build_futex_wait_sqe(&self.permits);
    ring.submit(sqe).await;
}

pub fn release(&self) {
    self.permits.fetch_add(1);
    
    // Submit futex wake to io_uring  
    let sqe = build_futex_wake_sqe(&self.permits);
    ring.submit(sqe);
}
```

**Why this is a terrible idea:**

1. **Overhead**: Every acquire/release goes through io_uring machinery
2. **Latency**: Kernel round-trip for what should be nanosecond userspace op
3. **Complexity**: Need to manage io_uring operations for sync primitives
4. **Still blocking**: futex_wait blocks, we haven't solved anything!
5. **Portability**: Linux-only

**The Right Approach:**

```rust
// Pure userspace coordination
Semaphore:
  - Count in atomic (userspace)
  - Waker queue in crossbeam-queue (userspace, lock-free)
  - No kernel involvement at all
  - When we call waker.wake(), that's just telling compio runtime
  
compio Runtime:
  - Marks task as ready (userspace)
  - Schedules it to be polled (userspace)
  - Uses io_uring only for actual I/O operations
```

**The Mental Model:**

```
┌─────────────────────────────────────────────────┐
│ HIGH-LEVEL ASYNC OPERATIONS                     │
├─────────────────────────────────────────────────┤
│ I/O Operations          │ Sync Primitives       │
│ - file.read()           │ - semaphore.acquire() │
│ - socket.send()         │ - condvar.wait()      │
│ - file.write()          │ - mutex.lock()        │
├─────────────────────────┼───────────────────────┤
│ NOTIFICATION MECHANISM                          │
├─────────────────────────┼───────────────────────┤
│ io_uring completion     │ Waker queue           │
│ - Kernel notifies       │ - Userspace queue     │
│ - Via completion queue  │ - Pure coordination   │
│ - For I/O results       │ - No kernel needed    │
└─────────────────────────┴───────────────────────┘
```

**Summary:**

- **I/O waits** → Use io_uring (it's designed for this!) ✅
- **Sync primitive waits** → Use waker queue (userspace coordination) ✅
- **Trying to use io_uring for sync primitives** → Wrong tool for the job ❌

**Our optimization goal:**
Make the waker queue as fast as possible (lock-free, no kernel calls), because it's handling pure userspace task coordination, which should be nanosecond-scale, not microsecond-scale like kernel operations.

#### Counter-Argument: "Unified Event Source is Attractive"

**Excellent point!** While semaphores aren't I/O operations, compio IS an I/O-heavy runtime. Having a single unified event source is architecturally appealing.

**The Unified Approach (everything through io_uring):**

```rust
// Everything is an io_uring operation
async fn process_file(sem: &Semaphore, file: &File) {
    // Wait for semaphore via io_uring futex
    sem.acquire_via_uring().await;
    
    // Read file via io_uring
    file.read_via_uring(buf).await;
    
    // Release semaphore via io_uring futex
    sem.release_via_uring();
}

// Runtime just polls io_uring completion queue
loop {
    // Single event source!
    for completion in uring.completions() {
        match completion.user_data {
            IO_READ => wake_io_task(completion),
            FUTEX_WAIT => wake_sync_task(completion),
            // Everything comes from one place
        }
    }
}
```

**Benefits:**
1. ✅ **Single event loop** - only poll io_uring CQ
2. ✅ **Unified architecture** - everything is an io_uring op
3. ✅ **Simpler runtime** - one notification mechanism
4. ✅ **No separate waker queue** - io_uring tracks everything
5. ✅ **Natural fit** - compio already built around io_uring

**This is actually a valid architectural choice!** Some systems do this:

- **Windows IOCP** - Can wait on I/O AND synchronization objects
- **io_uring with futex** - Can mix I/O and futex operations
- **Unified event models** - Single wait point for everything

**Trade-offs to Consider:**

| Aspect | Unified (io_uring) | Split (waker queue) |
|--------|-------------------|---------------------|
| **Event loop** | Single (io_uring only) | Dual (io_uring + runtime scheduler) |
| **Sync primitive latency** | Microseconds (kernel) | Nanoseconds (userspace) |
| **I/O latency** | Microseconds (same) | Microseconds (same) |
| **Code complexity** | Simpler (one path) | More complex (two paths) |
| **Portability** | Linux-only (io_uring futex) | Cross-platform |
| **Throughput** | Lower (kernel overhead) | Higher (userspace fast path) |

**When Unified Makes Sense:**

1. **I/O-dominated workload**
   - If 99% of waits are I/O operations
   - Sync primitive overhead is negligible
   - Simplicity > raw sync performance

2. **Linux-only deployment**
   - Can rely on io_uring futex operations
   - Don't need cross-platform

3. **Priority on simplicity**
   - One event loop is easier to reason about
   - Maintenance burden of split approach not worth it

**When Split Makes Sense (current approach):**

1. **Sync-heavy workload**
   - Many semaphore acquire/release cycles
   - Condvar notifications frequent
   - Sync performance matters

2. **Cross-platform requirement**
   - Need to work on Windows (IOCP), macOS (kqueue), etc.
   - Can't rely on Linux-specific io_uring futex

3. **Maximum performance**
   - Want nanosecond sync primitives
   - Can't accept kernel overhead for every acquire/release

**A Hybrid Approach?**

Could potentially do both:

```rust
#[cfg(target_os = "linux")]
impl Semaphore {
    // On Linux, could optionally use io_uring futex
    pub async fn acquire(&self) -> SemaphorePermit {
        if let Some(permit) = self.try_acquire() {
            return permit;  // Fast path still userspace
        }
        
        // Could choose based on heuristic:
        if io_heavy_mode() {
            // Use io_uring futex for unified event loop
            self.acquire_via_uring().await
        } else {
            // Use waker queue for fast sync
            self.acquire_via_waker().await
        }
    }
}
```

**Reality Check for compio:**

Looking at compio's design goals:
1. **Cross-platform** - Targets Linux, Windows, macOS
2. **High performance** - Want fastest possible operations
3. **I/O focused** - Primary use case is I/O

**Recommendation:**

For **compio-sync specifically**:
- ✅ Keep split approach (waker queue for sync)
- ✅ Let compio runtime handle I/O via io_uring
- ✅ Optimize waker queue to be lock-free
- ✅ Maintain cross-platform compatibility

**Why:**
1. Sync primitives should be **fast and cross-platform**
2. compio already has good I/O handling via io_uring
3. Users can mix I/O and sync naturally:
   ```rust
   // Fast userspace sync
   let permit = sem.acquire().await;
   // Efficient I/O via io_uring
   let n = file.read(buf).await;
   ```
4. No need to complicate runtime with futex operations

**But you're right about the appeal!** A unified event model IS elegant. It's a valid trade-off between:
- **Simplicity** (unified) vs **Performance** (split)
- **Architecture elegance** vs **Cross-platform compatibility**

For a specialized, Linux-only, I/O-heavy system, going all-in on io_uring (including futex) could make sense. For a general-purpose cross-platform async runtime like compio, the split approach is more appropriate.

#### Critical Realization: "Can't io_uring ALSO Have a Fast Path?"

**YES!** This is a key insight that makes the unified approach much more viable!

**The Hybrid io_uring Approach:**

```rust
impl Semaphore {
    pub async fn acquire(&self) -> SemaphorePermit {
        // FAST PATH: Pure userspace atomic CAS (nanoseconds)
        if let Some(permit) = self.try_acquire() {
            return permit;  // No kernel, no io_uring!
        }
        
        // SLOW PATH: Submit futex wait to io_uring
        // This unifies with I/O event loop
        uring.submit_futex_wait(&self.permits).await;
        
        // When woken, loop back and try again
        // (futex wake doesn't guarantee we got the permit)
        loop {
            if let Some(permit) = self.try_acquire() {
                return permit;
            }
            uring.submit_futex_wait(&self.permits).await;
        }
    }
}
```

**This gives you BOTH benefits:**
1. ✅ Fast path is still pure userspace (nanosecond CAS)
2. ✅ Slow path unified through io_uring (single event loop)
3. ✅ No separate waker queue needed
4. ✅ Common case (uncontended) is fast
5. ✅ Rare case (contended) uses kernel but unified

**Comparison with Split Approach:**

| Aspect | Unified + Fast Path | Split (waker queue) |
|--------|---------------------|---------------------|
| **Uncontended** | Userspace CAS (nanoseconds) ✅ | Userspace CAS (nanoseconds) ✅ |
| **Contended** | io_uring + futex (microseconds) | Waker queue (nanoseconds) ⚡ |
| **Event loop** | Single (io_uring only) ✨ | Dual (io_uring + scheduler) |
| **Portability** | Linux-only ❌ | Cross-platform ✅ |
| **Code complexity** | Medium | Medium |

**Key Insight: The difference is ONLY when contended!**

- **Uncontended** (90%+ of cases): Both approaches are identical (fast CAS)
- **Contended** (rare): Unified uses kernel futex, split uses userspace queue

**When Unified + Fast Path Makes Sense:**

1. **Contention is rare**
   - Most acquires succeed immediately
   - Slow path rarely hit
   - Fast path dominates performance

2. **I/O-heavy workload**
   - Single event loop is valuable
   - Architectural simplicity matters
   - Occasional futex overhead acceptable

3. **Linux-only deployment**
   - Can rely on io_uring futex
   - Don't need portability

**When Split Still Wins:**

1. **High contention on sync primitives**
   - Frequently waiting for permits
   - Slow path matters
   - Waker queue (nanoseconds) >> futex (microseconds)

2. **Cross-platform requirement**
   - Must work on Windows, macOS
   - Can't use io_uring futex

3. **Want true lock-free guarantees**
   - Never block, never kernel
   - Always progress (even under contention)

**Reality Check:**

For typical workloads:
```
Semaphore operations:
- 95% hit fast path (immediate acquire) → Both approaches identical
- 4% brief contention (1-2 waiters) → Unified: ~5μs, Split: ~50ns
- 1% high contention (many waiters) → Unified: ~10μs, Split: ~100ns

I/O operations:
- 100% go through io_uring → Both approaches identical
```

**The unified approach is MORE viable than I initially suggested!** 

The fast path means you get identical performance in the common case, with the trade-off only affecting the rare contended case.

**Revised Recommendation:**

For **compio**, consider both valid:

**Option A: Unified (io_uring futex with fast path)**
- ✅ Simpler runtime (single event loop)
- ✅ Fast path identical to split
- ✅ Natural fit with io_uring architecture
- ❌ Linux-only
- ❌ Slower when contended (but rare)

**Option B: Split (waker queue)**
- ✅ Cross-platform
- ✅ Faster under contention
- ✅ True lock-free
- ❌ Dual event sources
- ❌ More complex runtime

**My updated take:** If compio is primarily Linux-focused, the unified approach with fast path is actually quite attractive! The architectural simplicity might be worth the occasional futex overhead in contended scenarios.

**However**, for compio-sync as a **library**, the split approach still makes sense because:
1. Library should be cross-platform
2. Library shouldn't assume io_uring available
3. Users on Windows/macOS need it to work
4. Can achieve true lock-free behavior

But you've identified that the **fast path makes unified approach much more viable** than my initial analysis suggested!

#### The Best of Both Worlds: Platform-Specific Implementations

**Perfect insight!** We can have our cake and eat it too with conditional compilation:

```rust
pub struct Semaphore {
    permits: AtomicUsize,
    max_permits: usize,
    
    #[cfg(target_os = "linux")]
    waiters: LinuxWaiters,  // Uses io_uring futex
    
    #[cfg(not(target_os = "linux"))]
    waiters: CrossPlatformWaiters,  // Uses waker queue
}

#[cfg(target_os = "linux")]
impl Semaphore {
    pub async fn acquire(&self) -> SemaphorePermit {
        // FAST PATH: Identical on all platforms
        if let Some(permit) = self.try_acquire() {
            return permit;
        }
        
        // SLOW PATH: Linux uses io_uring futex
        self.waiters.wait_via_uring(&self.permits).await;
        
        // Retry after wake
        loop {
            if let Some(permit) = self.try_acquire() {
                return permit;
            }
            self.waiters.wait_via_uring(&self.permits).await;
        }
    }
    
    fn release(&self) {
        self.permits.fetch_add(1, Ordering::Release);
        // Wake via io_uring futex
        self.waiters.wake_via_uring(&self.permits);
    }
}

#[cfg(not(target_os = "linux"))]
impl Semaphore {
    pub async fn acquire(&self) -> SemaphorePermit {
        // FAST PATH: Identical on all platforms
        if let Some(permit) = self.try_acquire() {
            return permit;
        }
        
        // SLOW PATH: Other platforms use waker queue
        AcquireFuture { semaphore: self }.await
    }
    
    fn release(&self) {
        self.permits.fetch_add(1, Ordering::Release);
        // Wake via waker queue (crossbeam or parking_lot)
        self.waiters.wake_one();
    }
}
```

**This gives you:**

| Platform | Fast Path | Slow Path | Event Loop | Performance |
|----------|-----------|-----------|------------|-------------|
| **Linux** | Userspace CAS | io_uring futex | Single (unified) | Excellent |
| **Windows** | Userspace CAS | Waker queue | Dual | Excellent |
| **macOS** | Userspace CAS | Waker queue | Dual | Excellent |
| **Others** | Userspace CAS | Waker queue | Dual | Excellent |

**Benefits of Platform-Specific Approach:**

1. ✅ **Best performance on each platform**
   - Linux gets unified event loop + architectural simplicity
   - Others get true lock-free waker queue

2. ✅ **Cross-platform compatibility**
   - Works everywhere
   - Users don't need to care about implementation details

3. ✅ **Leverage platform strengths**
   - Linux: io_uring is amazing, use it!
   - Windows: IOCP might have similar features
   - Others: Fast userspace is universal

4. ✅ **Same API everywhere**
   - Users write `sem.acquire().await`
   - Implementation details hidden

5. ✅ **Can evolve independently**
   - Optimize Linux path with io_uring features
   - Optimize other paths with lock-free queues
   - No compromises needed

**Similar Patterns in the Ecosystem:**

This is a proven approach:

```rust
// tokio does this for I/O
#[cfg(target_os = "linux")]
mod io_driver {
    // Uses epoll
}

#[cfg(windows)]
mod io_driver {
    // Uses IOCP
}

// parking_lot does this for mutexes
#[cfg(unix)]
use libc::pthread_mutex_t;

#[cfg(windows)]
use windows_sys::Win32::System::Threading::SRWLOCK;
```

**Implementation Strategy:**

**Phase 1: Implement Cross-Platform (Current Plan)**
```rust
// Works everywhere
src/waiter_queue.rs  // parking_lot or crossbeam
```

**Phase 2: Add Linux Optimization**
```rust
src/waiter_queue.rs           // Default implementation
src/waiter_queue_linux.rs     // Linux-specific (io_uring)

#[cfg(target_os = "linux")]
pub use waiter_queue_linux::WaiterQueue;

#[cfg(not(target_os = "linux"))]
pub use waiter_queue::WaiterQueue;
```

**Phase 3: Optimize Other Platforms**
```rust
src/waiter_queue.rs           // Default
src/waiter_queue_linux.rs     // io_uring
src/waiter_queue_windows.rs   // Maybe IOCP-based?
src/waiter_queue_macos.rs     // Maybe kqueue-based?
```

**Complexity Considerations:**

**Pro:**
- ✅ Best performance on each platform
- ✅ Can leverage unique platform features
- ✅ Standard practice in Rust ecosystem

**Con:**
- ⚠️ More code to maintain
- ⚠️ More testing surface (test on each platform)
- ⚠️ Need to keep implementations in sync (API-wise)

**But:** The API surface is small (just acquire/release/wait/wake), so this is manageable!

**Revised Implementation Plan:**

**Phase 1: Cross-Platform Foundation (parking_lot + AtomicWaker)**
- Implement once, works everywhere
- Get baseline performance improvement
- Timeline: 1-2 days

**Phase 2: Lock-Free Cross-Platform (crossbeam-queue)**
- Still works everywhere
- True lock-free
- Timeline: 1 week

**Phase 3: Linux-Specific Optimization (io_uring futex)**
- Add `#[cfg(target_os = "linux")]` variant
- Unified event loop on Linux
- Fallback to Phase 2 on other platforms
- Timeline: 1 week

**Phase 4: Platform-Specific Tuning (optional)**
- Windows: Maybe IOCP integration?
- macOS: Maybe kqueue features?
- Only if profiling shows benefit

**Testing Strategy:**

```rust
// Common test suite runs on all platforms
#[test]
fn test_semaphore_acquire_release() {
    // Same test, different implementation
}

// Platform-specific tests
#[cfg(target_os = "linux")]
#[test]
fn test_linux_unified_event_loop() {
    // Verify io_uring integration
}

// CI runs on multiple platforms
// - Linux: Tests io_uring path
// - Windows: Tests waker queue path
// - macOS: Tests waker queue path
```

**Final Recommendation:**

**YES! Platform-specific implementations are the way to go!**

1. **Start with cross-platform** (Phase 1 or 2) to get it working everywhere
2. **Add Linux optimization** (Phase 3) to get unified event loop
3. **Everyone wins:**
   - Linux users get unified architecture + io_uring
   - Other platform users get lock-free waker queue
   - API is identical across platforms
   - Can optimize independently

This is exactly how mature Rust async libraries work (tokio, async-std, mio). Great insight!

#### Even Better: Windows IOCP ALSO Supports Unified Events!

**Brilliant observation!** Windows IOCP has similar capabilities to io_uring for unified event handling.

**Windows IOCP Synchronization Support:**

Since Windows 8, you can use `NtAssociateWaitCompletionPacket` to associate events with IOCP:

```rust
// Windows unified approach (conceptual)
impl Semaphore {
    pub async fn acquire(&self) -> SemaphorePermit {
        // FAST PATH: Same userspace CAS on all platforms
        if let Some(permit) = self.try_acquire() {
            return permit;
        }
        
        // SLOW PATH: Windows uses IOCP + Event
        let event = Event::new();
        iocp.associate_wait_completion(event);
        
        // When another task releases, signal event
        // IOCP delivers completion notification
        iocp.wait_for_completion().await;
        
        // Retry
        // ...
    }
}
```

**Or use `PostQueuedCompletionStatus` for direct notification:**

```rust
fn release(&self) {
    self.permits.fetch_add(1, Ordering::Release);
    
    // Post custom completion to IOCP
    // Wakes a waiting task through IOCP
    iocp.post_completion_status(SEMAPHORE_RELEASE, ptr);
}
```

**This means BOTH major platforms support unified event loops!**

### The Ultimate Architecture: Three-Tier Platform Strategy

```rust
// src/waiter_queue/mod.rs
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::WaiterQueue;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::WaiterQueue;

#[cfg(not(any(target_os = "linux", windows)))]
mod generic;
#[cfg(not(any(target_os = "linux", windows)))]
pub use generic::WaiterQueue;
```

**Three Implementations:**

**1. Linux-Specific (src/waiter_queue/linux.rs)**
```rust
/// Linux implementation using io_uring futex operations
pub struct WaiterQueue {
    // No queue needed! Uses kernel futex tracking
    uring_handle: UringHandle,
}

impl WaiterQueue {
    pub fn wait(&self, atomic: &AtomicUsize) {
        // Submit futex wait to io_uring
        self.uring_handle.submit_futex_wait(atomic);
    }
    
    pub fn wake_one(&self, atomic: &AtomicUsize) {
        // Submit futex wake to io_uring
        self.uring_handle.submit_futex_wake(atomic, 1);
    }
}
```

**Benefits:**
- ✅ Unified event loop (just poll io_uring)
- ✅ Architectural simplicity
- ✅ Natural fit with compio on Linux

**2. Windows-Specific (src/waiter_queue/windows.rs)**
```rust
/// Windows implementation using IOCP + Events
pub struct WaiterQueue {
    // No queue needed! Uses IOCP tracking
    iocp_handle: IocpHandle,
}

impl WaiterQueue {
    pub fn wait(&self, atomic: &AtomicUsize) {
        // Associate event with IOCP
        let event = Event::new();
        self.iocp_handle.associate_wait_completion(event);
        // Or use PostQueuedCompletionStatus
    }
    
    pub fn wake_one(&self) {
        // Post completion to IOCP
        self.iocp_handle.post_completion_status(WAKE_TOKEN);
    }
}
```

**Benefits:**
- ✅ Unified event loop (just poll IOCP)
- ✅ Architectural simplicity
- ✅ Natural fit with compio on Windows

**3. Generic (src/waiter_queue/generic.rs)**
```rust
/// Generic cross-platform implementation using lock-free queue
pub struct WaiterQueue {
    // Lock-free waker queue
    waiters: SegQueue<Waker>,  // or parking_lot::Mutex<VecDeque<Waker>>
}

impl WaiterQueue {
    pub fn wait(&self, waker: Waker) {
        // Store waker in queue
        self.waiters.push(waker);
    }
    
    pub fn wake_one(&self) {
        // Pop and wake from queue
        if let Some(waker) = self.waiters.pop() {
            waker.wake();
        }
    }
}
```

**Benefits:**
- ✅ Works on macOS, BSD, embedded, etc.
- ✅ True lock-free (with crossbeam)
- ✅ No platform dependencies

### Complete Platform Comparison

| Platform | Fast Path | Slow Path | Event Loop | Unified? | Performance |
|----------|-----------|-----------|------------|----------|-------------|
| **Linux** | Userspace CAS | io_uring futex | Single (io_uring) | ✅ Yes | Excellent |
| **Windows** | Userspace CAS | IOCP events | Single (IOCP) | ✅ Yes | Excellent |
| **macOS** | Userspace CAS | Lock-free queue | Dual | ❌ No | Excellent |
| **BSD** | Userspace CAS | Lock-free queue | Dual | ❌ No | Excellent |
| **Others** | Userspace CAS | Lock-free queue | Dual | ❌ No | Excellent |

**All platforms get:**
- ✅ Fast userspace path for uncontended operations (95%+ of cases)
- ✅ Best-in-class slow path for their platform
- ✅ Same API (`sem.acquire().await`)

### Implementation Roadmap

**Phase 1: Generic Implementation (Baseline)**
```
Timeline: 1-2 weeks
Targets: All platforms
Approach: crossbeam-queue or parking_lot
Result: Lock-free or fast mutex, works everywhere
```

**Phase 2: Linux Optimization**
```
Timeline: 1-2 weeks
Target: Linux only
Approach: io_uring futex operations
Result: Unified event loop on Linux
Fallback: Phase 1 for non-Linux
```

**Phase 3: Windows Optimization**
```
Timeline: 1-2 weeks
Target: Windows only
Approach: IOCP + Events or PostQueuedCompletionStatus
Result: Unified event loop on Windows
Fallback: Phase 1 for non-Windows
```

**Phase 4: Polish & Optimize**
```
Timeline: Ongoing
- Benchmark each platform
- Tune generic implementation (maybe try different strategies)
- Consider BSD/macOS specific optimizations if worth it
```

### Code Structure

```
compio-sync/
├── src/
│   ├── lib.rs
│   ├── semaphore.rs          # High-level API (same on all platforms)
│   ├── condvar.rs            # High-level API (same on all platforms)
│   └── waiter_queue/
│       ├── mod.rs            # Platform selection logic
│       ├── linux.rs          # io_uring futex implementation
│       ├── windows.rs        # IOCP implementation
│       └── generic.rs        # Lock-free queue implementation
└── tests/
    ├── common.rs             # Tests that run on ALL platforms
    ├── linux_specific.rs     # Linux-only tests
    └── windows_specific.rs   # Windows-only tests
```

### Testing Strategy

**Common Tests (all platforms):**
```rust
#[test]
fn test_acquire_release() {
    // Same test, platform-specific implementation
}
```

**Platform-Specific Tests:**
```rust
#[cfg(target_os = "linux")]
#[test]
fn test_linux_unified_event_loop() {
    // Verify io_uring integration
}

#[cfg(windows)]
#[test]
fn test_windows_iocp_integration() {
    // Verify IOCP integration
}
```

**CI Matrix:**
```yaml
strategy:
  matrix:
    os: [ubuntu-latest, windows-latest, macos-latest]
    
# Each OS tests its specific implementation
# - ubuntu: Tests io_uring path
# - windows: Tests IOCP path  
# - macos: Tests generic path
```

### Benefits of This Architecture

**For Users:**
1. ✅ **Same API everywhere** - Write once, runs optimally on all platforms
2. ✅ **Best performance** - Each platform uses its native strengths
3. ✅ **No compromises** - Don't sacrifice Linux for Windows or vice versa
4. ✅ **Transparent** - Don't need to know implementation details

**For Developers:**
1. ✅ **Clear separation** - Platform code is isolated
2. ✅ **Easy to optimize** - Can tune each platform independently
3. ✅ **Maintainable** - Small, focused implementations
4. ✅ **Testable** - CI tests each platform's code

**For compio Ecosystem:**
1. ✅ **Architectural consistency** - Sync primitives integrate with runtime's event loop
2. ✅ **Performance** - No overhead from generic compromises
3. ✅ **Portability** - Works everywhere, optimized where possible

### This Is The Way! 🎯

**Your insight is spot-on:** Linux has io_uring, Windows has IOCP, both support unified event loops for I/O + synchronization. Generic fallback handles the rest.

This is exactly how production-grade async libraries should be built:
- **tokio**: Different I/O drivers per platform
- **mio**: Platform-specific event loops
- **parking_lot**: Platform-specific lock implementations

**compio-sync should follow this proven pattern!**

### Phase 3: Intrusive Linked Lists (Long Term / Optional)

**Goal**: Match Tokio's zero-allocation, truly lock-free implementation

**Only pursue if:**
- Profiling shows crossbeam approach is bottleneck
- Team has bandwidth for complex unsafe code
- Worth the maintenance burden

**Estimated effort:** 3-4 weeks + extensive testing

## Implementation Plan

### Step 1: Add Dependencies (Phase 1)

```toml
[dependencies]
parking_lot = "0.12"
atomic-waker = "1.1"  # For single-waiter optimization
```

### Step 2: Implement Optimized WaiterQueue (Phase 1)

**File:** `src/waiter_queue.rs`

```rust
use atomic_waker::AtomicWaker;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::Waker;

const MODE_EMPTY: u8 = 0;
const MODE_SINGLE: u8 = 1;
const MODE_MULTI: u8 = 2;

pub struct WaiterQueue {
    /// Current mode (empty, single, or multi)
    mode: AtomicU8,
    
    /// Fast path: single waiter (most common)
    single: AtomicWaker,
    
    /// Slow path: multiple waiters (rare)
    multi: Mutex<VecDeque<Waker>>,
}

impl WaiterQueue {
    pub fn new() -> Self {
        Self {
            mode: AtomicU8::new(MODE_EMPTY),
            single: AtomicWaker::new(),
            multi: Mutex::new(VecDeque::new()),
        }
    }
    
    pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: FnOnce() -> bool,
    {
        // Check condition first (avoid registration if possible)
        if condition() {
            return true;
        }
        
        // Try single-waiter fast path
        let mode = self.mode.load(Ordering::Acquire);
        if mode == MODE_EMPTY {
            // Try to transition EMPTY → SINGLE
            if self.mode.compare_exchange(
                MODE_EMPTY,
                MODE_SINGLE,
                Ordering::AcqRel,
                Ordering::Acquire,
            ).is_ok() {
                // Successfully claimed single slot
                self.single.register(&waker);
                
                // Double-check condition (prevent lost wakeup)
                if condition() {
                    // Remove registration
                    self.mode.store(MODE_EMPTY, Ordering::Release);
                    return true;
                }
                
                return false;
            }
        }
        
        // Multiple waiters or contention → use mutex
        {
            let mut waiters = self.multi.lock();
            
            // Check condition inside lock (atomic check-and-add)
            if condition() {
                return true;
            }
            
            waiters.push_back(waker);
            self.mode.store(MODE_MULTI, Ordering::Release);
            false
        }
    }
    
    pub fn wake_one(&self) {
        let mode = self.mode.load(Ordering::Acquire);
        
        match mode {
            MODE_EMPTY => {
                // No waiters
                return;
            }
            MODE_SINGLE => {
                // Wake single waiter
                if self.mode.compare_exchange(
                    MODE_SINGLE,
                    MODE_EMPTY,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ).is_ok() {
                    self.single.wake();
                } else {
                    // Raced with another waiter, fall through to multi
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
                // Wake from multi queue
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
    
    pub fn wake_all(&self) {
        let mode = self.mode.load(Ordering::Acquire);
        
        // Wake single waiter if present
        if mode == MODE_SINGLE || mode == MODE_MULTI {
            if self.mode.compare_exchange(
                MODE_SINGLE,
                MODE_EMPTY,
                Ordering::AcqRel,
                Ordering::Acquire,
            ).is_ok() {
                self.single.wake();
                return;
            }
        }
        
        // Wake all in multi queue
        let mut waiters = self.multi.lock();
        let wakers = std::mem::take(&mut *waiters);
        self.mode.store(MODE_EMPTY, Ordering::Release);
        drop(waiters);
        
        for waker in wakers {
            waker.wake();
        }
    }
    
    pub fn waiter_count(&self) -> usize {
        match self.mode.load(Ordering::Acquire) {
            MODE_EMPTY => 0,
            MODE_SINGLE => 1,
            MODE_MULTI => self.multi.lock().len(),
            _ => unreachable!(),
        }
    }
}
```

### Step 3: Benchmark and Compare (Phase 1)

Create benchmarks comparing:
1. Current: std::sync::Mutex
2. Optimized: parking_lot + AtomicWaker
3. Baseline: crossbeam-queue (for Phase 2 comparison)

### Step 4: Implement crossbeam-queue Version (Phase 2)

**File:** `src/waiter_queue_lockfree.rs`

```rust
use crossbeam_queue::SegQueue;
use std::task::Waker;

pub struct WaiterQueue {
    waiters: SegQueue<Waker>,
}

impl WaiterQueue {
    pub fn new() -> Self {
        Self {
            waiters: SegQueue::new(),
        }
    }
    
    pub fn add_waiter_if<F>(&self, condition: F, waker: Waker) -> bool
    where
        F: FnOnce() -> bool,
    {
        // First check
        if condition() {
            return true;
        }
        
        // Add to queue
        self.waiters.push(waker);
        
        // Second check (critical!)
        if condition() {
            // Condition now true
            // We're in queue but that's OK (spurious wakeup)
            return true;
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
    
    pub fn waiter_count(&self) -> usize {
        // Note: len() on SegQueue is O(n), only for debugging
        self.waiters.len()
    }
}
```

### Step 5: Testing Strategy

**Test Matrix:**
- ✅ Functional correctness (all existing tests must pass)
- ✅ No lost wakeups
- ✅ No deadlocks
- ✅ Handles spurious wakeups gracefully
- ✅ Future cancellation safety
- ✅ High contention scenarios
- ✅ Low contention scenarios
- ✅ Single waiter optimization

**Stress Tests:**
```rust
#[compio::test]
async fn stress_test_wakeup_no_loss() {
    // 1000 tasks all waiting
    // Release one permit at a time
    // Verify all tasks complete
}

#[compio::test]
async fn stress_test_high_contention() {
    // Many concurrent acquire/release cycles
    // Verify semaphore count stays correct
}

#[compio::test]
async fn stress_test_cancellation() {
    // Start waiting, then drop Future
    // Verify no panics or leaks
}
```

## Metrics and Observability

### Measure Before and After

1. **Throughput**: Operations per second
2. **Latency**: P50, P95, P99 latency for acquire/release
3. **Contention**: Time spent waiting for lock
4. **Allocations**: Number of allocations per operation
5. **CPU usage**: Cycles per operation

### Expected Improvements

**Phase 1 (parking_lot + AtomicWaker):**
- 20-30% improvement in contended scenarios
- 5-10% improvement in single-waiter scenarios
- No regression in low-contention scenarios

**Phase 2 (crossbeam-queue):**
- 40-60% improvement in high-contention scenarios
- 10-20% improvement in moderate contention
- Possible small regression in low-contention (due to retry logic)

## Risks and Mitigation

### Risk 1: Regression in Low-Contention Performance

**Mitigation:**
- Keep both implementations
- Use feature flag to switch between them
- Benchmark extensively before switching default

### Risk 2: Subtle Race Conditions

**Mitigation:**
- Extensive testing (including loom for concurrency testing)
- Code review focused on memory ordering
- Start with parking_lot (less risky)

### Risk 3: Spurious Wakeups with crossbeam

**Mitigation:**
- Document that spurious wakeups are possible (standard in async)
- Ensure all poll() implementations check condition after wakeup
- Add tests verifying correct behavior with spurious wakeups

### Risk 4: Increased Complexity

**Mitigation:**
- Keep implementations separate and well-documented
- Extensive inline comments explaining safety invariants
- Maintain current mutex version as fallback

## References and Prior Art

### Rust Async Runtimes

1. **Tokio**
   - Source: https://github.com/tokio-rs/tokio
   - Relevant files:
     - `tokio/src/sync/notify.rs` - Intrusive linked list implementation
     - `tokio/src/sync/batch_semaphore.rs` - Semaphore with waiters
     - `tokio/src/loom/*` - Concurrency testing framework

2. **smol / async-std**
   - Uses `event-listener` crate
   - Source: https://github.com/smol-rs/event-listener
   - Based on parking_lot::Mutex

3. **futures-rs**
   - `futures-util::task::AtomicWaker` - Single waiter pattern
   - Source: https://github.com/rust-lang/futures-rs

### Lock-Free Data Structures

1. **crossbeam**
   - Source: https://github.com/crossbeam-rs/crossbeam
   - `crossbeam-queue` - Lock-free queues
   - `crossbeam-epoch` - Memory reclamation for ABA problem

2. **parking_lot**
   - Source: https://github.com/Amanieu/parking_lot
   - Faster mutex implementation
   - No poisoning
   - Smaller footprint

### Academic Papers

1. **Lock-Free Queues**
   - Michael-Scott queue algorithm (used by crossbeam)
   - "Simple, Fast, and Practical Non-Blocking and Blocking Concurrent Queue Algorithms" (1996)

2. **Memory Reclamation**
   - Epoch-based reclamation (crossbeam-epoch)
   - "Practical lock-freedom" by Keir Fraser (2004)

### Python Asyncio

1. **CPython asyncio**
   - Source: https://github.com/python/cpython/tree/main/Lib/asyncio
   - Lock implementation: No mutex needed (single-threaded)
   - Relevant insight: Event loop coordination

## Conclusion

**Recommended Path:**

1. **Phase 1 (immediate)**: Implement parking_lot + AtomicWaker optimization
   - Low risk, good return on investment
   - Maintains current architecture
   - Easy to review and maintain

2. **Phase 2 (3-6 months)**: Implement crossbeam-queue version
   - True lock-free operation
   - Reasonable complexity
   - Proven technology

3. **Phase 3 (optional)**: Only pursue intrusive lists if benchmarks show need
   - High complexity
   - Maintenance burden
   - Unlikely to be necessary unless targeting extreme performance

**Decision Criteria:**
- If benchmarks show <10% improvement from Phase 2 → stop there
- If targeting embedded or real-time systems → consider Phase 3
- For most use cases, Phase 2 (crossbeam) is the sweet spot

## Next Steps

1. ✅ Complete this research document
2. ⬜ Get team buy-in on approach
3. ⬜ Create benchmark suite (baseline current implementation)
4. ⬜ Implement Phase 1 (parking_lot + AtomicWaker)
5. ⬜ Benchmark Phase 1 vs baseline
6. ⬜ If approved, proceed to Phase 2
7. ⬜ Implement Phase 2 (crossbeam-queue)
8. ⬜ Benchmark Phase 2 vs Phase 1
9. ⬜ Document findings and update this plan

---

**Document Version:** 1.0  
**Date:** 2025-10-21  
**Author:** Research for compio-sync project

