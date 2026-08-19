//! Cache module — durable cache entries, atomic writes, and active
//! locks.
//!
//! See the module-level docs in [`store::Cache`] for the directory
//! layout and the two-phase commit pipeline.
//!
//! Public re-exports are intentional: the IPC layer uses them, the
//! shelf module uses them, and the integration tests use them. Anything
//! not re-exported here is internal to the cache.

pub mod atomic;
pub mod locks;
pub mod store;

pub use atomic::{write_atomic, AtomicWriteOutcome};
pub use locks::{ActiveLockSet, CacheResult, CleanupOutcome, DismissOutcome, LockGuard};
pub use store::{Cache, CacheCommitRequest, CacheError, CommitResult, CommitStage};
