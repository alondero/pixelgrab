//! Pin module. Owns the registry, the lock guard, and the wiring into the
//! Tauri IPC layer. The registry is the single owner of the per-pin
//! state; the IPC layer is the only place that touches it from the
//! outside.
//!
//! The lock provider is pluggable so the production Windows build can wire
//! the shelf's cache lock in, while the synthetic and test paths use the
//! in-memory implementation.

pub mod lock;
pub mod registry;
pub mod window;

pub use lock::{CachePinLockProvider, InMemoryPinLockProvider, PinLockGuard};
pub use registry::{PinEntry, PinRegistry, MAX_PINS};
