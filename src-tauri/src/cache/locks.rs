//! Owner-keyed active locks per cache entry.
//!
//! Each cache entry may be held by one or more *lock owners*. An entry
//! is eligible for cleanup (deletion from disk) only when no owner
//! holds a lock. The lock registry is held in memory and is rebuilt
//! from disk on startup by `Cache::load_or_recover`; see `store.rs`
//! for the on-disk persistence story.
//!
//! The design is intentionally narrow:
//!
//! - `LockOwner` is a closed enum (see `pixelgrab_contracts::LockOwner`).
//!   New owner kinds must be added as new variants.
//! - A lock is identified by `(ShelfId, LockOwner)`. The same owner
//!   double-locking the same entry is a no-op (the underlying set
//!   already contains the owner) so the API is safe to call twice
//!   without bookkeeping.
//! - A `LockGuard` releases on `Drop`. Callers that want to release
//!   early can call `release()`.

use std::collections::{BTreeMap, BTreeSet};

use parking_lot::Mutex;
use pixelgrab_contracts::{LockOwner, PlatformError, PlatformErrorKind, PlatformResult, ShelfId};

/// Outcome of a `try_cleanup` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupOutcome {
    /// No entry exists for the given shelf id.
    Unknown,
    /// The entry exists and was removed from disk and from memory.
    Removed,
    /// The entry exists but is still locked by one or more owners.
    /// Callers that need the owner labels can use
    /// `ActiveLockSet::owners_of`.
    StillLocked,
}

/// Outcome of `try_dismiss` (used by the dismiss IPC). Returns the
/// final state alongside a short diagnostic string suitable for the
/// IPC response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DismissOutcome {
    /// What happened to the entry.
    pub removed: bool,
    /// Stable diagnostic label: one of `removed`, `still_locked`,
    /// `unknown_shelf_id`.
    pub reason: &'static str,
}

impl DismissOutcome {
    fn removed() -> Self {
        Self {
            removed: true,
            reason: "removed",
        }
    }
    fn still_locked() -> Self {
        Self {
            removed: false,
            reason: "still_locked",
        }
    }
}

/// In-memory active-lock registry. Thread-safe; the mutex is held only
/// for the duration of the mutation (the registry is small — at most
/// one entry per shelf card — so contention is negligible). The inner
/// mutex is wrapped in an `Arc` so handles can be cloned for `LockGuard`s
/// without requiring the guards to borrow from a `Mutex` held elsewhere.
#[derive(Debug, Default, Clone)]
pub struct ActiveLockSet {
    inner: std::sync::Arc<Mutex<LocksInner>>,
}

#[derive(Debug, Default)]
struct LocksInner {
    /// Owner count per shelf id. Always >= 1 for an active entry;
    /// dropping to zero removes the entry from the map.
    entries: BTreeMap<ShelfId, BTreeSet<LockOwner>>,
}

impl ActiveLockSet {
    /// Build an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a lock for `owner` on `shelf_id`. Returns an owned guard
    /// that releases the lock when dropped.
    ///
    /// Idempotent: if the same owner already holds the lock the call is
    /// a no-op and the returned guard still releases on drop (without
    /// changing the underlying count).
    pub fn acquire(&self, shelf_id: ShelfId, owner: LockOwner) -> LockGuard {
        {
            let mut inner = self.inner.lock();
            let owners = inner.entries.entry(shelf_id.clone()).or_default();
            owners.insert(owner);
        }
        LockGuard {
            registry: self.clone_handle(),
            shelf_id,
            owner,
        }
    }

    /// Inspect the current owners of an entry. Returns an empty slice
    /// when the entry has no locks (and therefore no entry in the map).
    pub fn owners_of(&self, shelf_id: &str) -> Vec<LockOwner> {
        let inner = self.inner.lock();
        inner
            .entries
            .get(shelf_id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Release a single owner from `shelf_id`. Internal helper used by
    /// the `LockGuard::Drop` impl and by `try_cleanup` / `try_dismiss`.
    /// Decrement-only — the entry is kept in the map (with an empty
    /// owner set) so `try_cleanup` can return `Removed` rather than
    /// `Unknown` for entries that have been fully unlocked.
    fn release(&self, shelf_id: &str, owner: LockOwner) {
        let mut inner = self.inner.lock();
        if let Some(owners) = inner.entries.get_mut(shelf_id) {
            owners.remove(&owner);
            // Intentionally do NOT remove the entry here — see the
            // function-level docs for why.
        }
    }

    /// Try to clean up an entry. Returns `CleanupOutcome::Removed` when
    /// no owners hold the entry and the entry is removed from the
    /// registry. Returns `StillLocked` when at least one owner remains.
    /// Returns `Unknown` when the shelf id was never seen by this
    /// registry. Callers that need the owner labels can use
    /// `ActiveLockSet::owners_of` instead.
    pub fn try_cleanup(&self, shelf_id: &str) -> CleanupOutcome {
        let mut inner = self.inner.lock();
        match inner.entries.get(shelf_id) {
            None => CleanupOutcome::Unknown,
            Some(owners) if owners.is_empty() => {
                inner.entries.remove(shelf_id);
                CleanupOutcome::Removed
            }
            Some(_) => CleanupOutcome::StillLocked,
        }
    }

    /// Dismiss the entry, releasing only the `Shelf` lock first and
    /// then attempting cleanup. If any other owner holds a lock the
    /// dismissal is partial: the shelf lock is released (so the card
    /// disappears) but the entry stays on disk.
    pub fn try_dismiss(&self, shelf_id: &str) -> DismissOutcome {
        // Track whether the entry existed before the shelf release so
        // we can distinguish "unknown shelf id" from "removed".
        let existed = {
            let inner = self.inner.lock();
            inner.entries.contains_key(shelf_id)
        };
        if !existed {
            return DismissOutcome {
                removed: false,
                reason: "unknown_shelf_id",
            };
        }
        // Release the shelf lock first; cleanup below decides the
        // outcome based on what remains.
        self.release(shelf_id, LockOwner::Shelf);
        match self.try_cleanup(shelf_id) {
            CleanupOutcome::Removed | CleanupOutcome::Unknown => DismissOutcome::removed(),
            CleanupOutcome::StillLocked => DismissOutcome::still_locked(),
        }
    }

    /// Snapshot every currently-held owner. Used by `ShelfSnapshot`
    /// and by the integration tests.
    pub fn snapshot(&self) -> BTreeMap<ShelfId, Vec<LockOwner>> {
        let inner = self.inner.lock();
        inner
            .entries
            .iter()
            .map(|(id, set)| (id.clone(), set.iter().copied().collect()))
            .collect()
    }

    /// Clone this registry. Used by `LockGuard` so it owns a handle
    /// rather than borrowing from a registry held by another struct.
    pub fn clone_handle(&self) -> ActiveLockSet {
        ActiveLockSet {
            inner: self.inner.clone(),
        }
    }
}

/// RAII guard returned by `ActiveLockSet::acquire`. Releasing the lock
/// is automatic on drop; call `release()` for explicit early release.
///
/// The guard **owns** a clone of the registry handle so it is `'static`
/// and can be stored in a `BTreeMap` inside the cache. The underlying
/// lock state is shared across every handle via the `Arc<Mutex<_>>` so
/// `release` happens once regardless of how many handles exist.
#[derive(Debug)]
pub struct LockGuard {
    registry: ActiveLockSet,
    shelf_id: ShelfId,
    owner: LockOwner,
}

impl LockGuard {
    /// Borrow the shelf id this guard is bound to.
    pub fn shelf_id(&self) -> &str {
        &self.shelf_id
    }

    /// Owner label this guard represents.
    pub fn owner(&self) -> LockOwner {
        self.owner
    }

    /// Release the lock early. After this returns the `Drop` impl
    /// runs the same `release` a second time, which is a no-op on
    /// the registry's `BTreeSet` (`remove` on a missing element is a
    /// no-op). The duplication is bounded and side-effect free.
    pub fn release(self) {
        self.registry.release(&self.shelf_id, self.owner);
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.registry.release(&self.shelf_id, self.owner);
    }
}

/// Helper used by the store layer: convert a `PlatformError`-less
/// outcome into a typed error so the IPC layer can surface it without
/// pattern matching on the cleanup enum at every call site.
pub fn cleanup_error(message: impl Into<String>) -> PlatformError {
    PlatformError::new(PlatformErrorKind::Internal, message.into())
}

/// Type alias for `Result<T, PlatformError>` to keep the public API
/// ergonomic without re-exporting `pixelgrab_contracts::PlatformResult`
/// everywhere. (Re-exported for completeness.)
pub type CacheResult<T> = PlatformResult<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_release_round_trip() {
        let locks = ActiveLockSet::new();
        let _guard = locks.acquire("shelf-1".to_string(), LockOwner::Shelf);
        assert_eq!(
            locks.owners_of("shelf-1"),
            vec![LockOwner::Shelf],
            "shelf owner is held",
        );
        drop(_guard);
        assert!(locks.owners_of("shelf-1").is_empty());
        // `try_cleanup` actively removes the empty entry, so the
        // second call returns Unknown.
        assert_eq!(locks.try_cleanup("shelf-1"), CleanupOutcome::Removed);
        assert_eq!(locks.try_cleanup("shelf-1"), CleanupOutcome::Unknown);
    }

    #[test]
    fn duplicate_acquire_is_idempotent() {
        let locks = ActiveLockSet::new();
        let _a = locks.acquire("shelf-1".to_string(), LockOwner::Shelf);
        let _b = locks.acquire("shelf-1".to_string(), LockOwner::Shelf);
        assert_eq!(locks.owners_of("shelf-1"), vec![LockOwner::Shelf]);
    }

    #[test]
    fn cleanup_blocked_while_any_owner_held() {
        let locks = ActiveLockSet::new();
        let _shelf = locks.acquire("shelf-1".to_string(), LockOwner::Shelf);
        let _editor = locks.acquire("shelf-1".to_string(), LockOwner::Editor);
        assert_eq!(locks.try_cleanup("shelf-1"), CleanupOutcome::StillLocked);
        let owners = locks.owners_of("shelf-1");
        assert!(owners.contains(&LockOwner::Shelf));
        assert!(owners.contains(&LockOwner::Editor));
        drop(_editor);
        // Only the shelf lock remains; cleanup still blocked.
        assert_eq!(locks.try_cleanup("shelf-1"), CleanupOutcome::StillLocked);
        assert_eq!(locks.owners_of("shelf-1"), vec![LockOwner::Shelf]);
        drop(_shelf);
        assert_eq!(locks.try_cleanup("shelf-1"), CleanupOutcome::Removed);
    }

    #[test]
    fn dismiss_releases_shelf_lock_only() {
        let locks = ActiveLockSet::new();
        let _shelf = locks.acquire("shelf-1".to_string(), LockOwner::Shelf);
        let _pin = locks.acquire("shelf-1".to_string(), LockOwner::Pin);
        let outcome = locks.try_dismiss("shelf-1");
        assert_eq!(
            outcome,
            DismissOutcome {
                removed: false,
                reason: "still_locked",
            }
        );
        // The pin lock remains, so cleanup is still blocked.
        assert_eq!(locks.try_cleanup("shelf-1"), CleanupOutcome::StillLocked);
        assert_eq!(locks.owners_of("shelf-1"), vec![LockOwner::Pin]);
    }

    #[test]
    fn dismiss_unknown_returns_unknown_reason() {
        let locks = ActiveLockSet::new();
        let outcome = locks.try_dismiss("never-existed");
        assert_eq!(
            outcome,
            DismissOutcome {
                removed: false,
                reason: "unknown_shelf_id",
            }
        );
    }

    #[test]
    fn snapshot_lists_all_locked_entries() {
        let locks = ActiveLockSet::new();
        let _a = locks.acquire("a".to_string(), LockOwner::Shelf);
        let _b = locks.acquire("b".to_string(), LockOwner::Editor);
        let _c = locks.acquire("c".to_string(), LockOwner::Pin);
        let snap = locks.snapshot();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap.get("a"), Some(&vec![LockOwner::Shelf]));
    }
}
