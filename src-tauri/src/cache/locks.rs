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
//! - A lock is identified by `(ShelfId, LockOwner)`. Acquiring the same
//!   `(shelf, owner)` pair more than once takes an additional
//!   *reference* (issue #63): each `acquire` increments a per-owner
//!   reference count and each guard drop decrements it, so N pins of
//!   the same capture keep the `Pin` owner alive until the last pin
//!   closes. The owner is only released when the count reaches zero.
//! - A `LockGuard` releases on `Drop`. Callers that want to release
//!   early can call `release()`; a guard releases its reference at most
//!   once even when both paths run.

use std::collections::BTreeMap;

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
    /// Reference count per `(shelf id, owner)` pair. Always >= 1 for an
    /// active owner; dropping to zero removes the owner from the map.
    /// The shelf entry itself is kept (with an empty map) so
    /// `try_cleanup` can return `Removed` rather than `Unknown`.
    entries: BTreeMap<ShelfId, BTreeMap<LockOwner, usize>>,
}

impl ActiveLockSet {
    /// Build an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a lock reference for `owner` on `shelf_id`. Returns an
    /// owned guard that releases the reference when dropped.
    ///
    /// Re-entrant: each call takes an additional reference on the
    /// `(shelf_id, owner)` pair (issue #63 pin refcounting). The owner
    /// stays visible to `owners_of` until every guard has dropped.
    pub fn acquire(&self, shelf_id: ShelfId, owner: LockOwner) -> LockGuard {
        {
            let mut inner = self.inner.lock();
            let owners = inner.entries.entry(shelf_id.clone()).or_default();
            *owners.entry(owner).or_insert(0) += 1;
        }
        LockGuard {
            registry: self.clone_handle(),
            shelf_id,
            owner,
            released: false,
        }
    }

    /// Inspect the current owners of an entry. Returns an empty slice
    /// when the entry has no locks (and therefore no entry in the map).
    pub fn owners_of(&self, shelf_id: &str) -> Vec<LockOwner> {
        let inner = self.inner.lock();
        inner
            .entries
            .get(shelf_id)
            .map(|owners| owners.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Release a single lock reference from `shelf_id`'s `owner`.
    /// Internal helper used by the `LockGuard::Drop` impl and by
    /// `try_cleanup` / `try_dismiss`. Decrement-only — the owner is
    /// removed when its reference count reaches zero, but the shelf
    /// entry is kept in the map (with an empty owner map) so
    /// `try_cleanup` can return `Removed` rather than `Unknown`.
    fn release(&self, shelf_id: &str, owner: LockOwner) {
        let mut inner = self.inner.lock();
        if let Some(owners) = inner.entries.get_mut(shelf_id) {
            if let Some(count) = owners.get_mut(&owner) {
                *count -= 1;
                if *count == 0 {
                    owners.remove(&owner);
                }
            }
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
            .map(|(id, owners)| (id.clone(), owners.keys().copied().collect()))
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
    /// `true` once this guard has released its reference (via the
    /// explicit `release()` method). The `Drop` impl skips the release
    /// when set, so a guard never decrements twice.
    released: bool,
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

    /// Release the lock reference early. After this returns the `Drop`
    /// impl is a no-op — each guard releases exactly one reference.
    pub fn release(mut self) {
        self.registry.release(&self.shelf_id, self.owner);
        self.released = true;
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.released {
            self.registry.release(&self.shelf_id, self.owner);
        }
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
    fn same_owner_guards_refcount_until_last_drop() {
        // Issue #63: two pins of the same capture each hold a guard.
        // The owner must stay visible until the LAST guard drops.
        let locks = ActiveLockSet::new();
        let a = locks.acquire("shelf-1".to_string(), LockOwner::Pin);
        let b = locks.acquire("shelf-1".to_string(), LockOwner::Pin);
        drop(a);
        assert_eq!(
            locks.owners_of("shelf-1"),
            vec![LockOwner::Pin],
            "one live reference must keep the owner"
        );
        drop(b);
        assert!(locks.owners_of("shelf-1").is_empty());
    }

    #[test]
    fn explicit_release_then_drop_decrements_once() {
        let locks = ActiveLockSet::new();
        let guard = locks.acquire("shelf-1".to_string(), LockOwner::Drag);
        guard.release();
        assert!(locks.owners_of("shelf-1").is_empty());
        // Dropping after an explicit release must not underflow the
        // reference count (the count is already at zero).
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
