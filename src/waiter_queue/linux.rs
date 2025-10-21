//! Linux-specific waiter queue implementation
//!
//! Phase 2 TODO: Will use io_uring futex operations for unified event loop
//! Phase 2 will add:
//! - io_uring futex operations (kernel 6.7+)
//! - Unified event loop (I/O + sync through io_uring)
//! - Runtime kernel detection with graceful fallback
//!
//! For now, re-exports the generic implementation (AtomicWaker + parking_lot).

// Phase 2: Re-export generic for now
pub use super::generic::WaiterQueue;
