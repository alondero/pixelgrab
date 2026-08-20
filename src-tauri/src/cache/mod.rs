//! Cache module — durable cache entries, atomic writes, active
//! locks, persistent policy, and the periodic sweeper.
//!
//! See the module-level docs in [`store::Cache`] for the directory
//! layout and the two-phase commit pipeline.
//!
//! Public re-exports are intentional: the IPC layer uses them, the
//! shelf module uses them, and the integration tests use them. Anything
//! not re-exported here is internal to the cache.

pub mod atomic;
pub mod locks;
pub mod policy;
pub mod store;
pub mod sweeper;

pub use atomic::{write_atomic, AtomicWriteOutcome};
pub use locks::{ActiveLockSet, CacheResult, CleanupOutcome, DismissOutcome, LockGuard};
pub use policy::{CachePolicyStore, BACKUP_FILENAME, PERSIST_DEBOUNCE, PRIMARY_FILENAME};
pub use store::{Cache, CacheCommitRequest, CacheError, CommitResult, CommitStage};
pub use sweeper::{CacheSweeper, SweepWorker};
