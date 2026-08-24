//! Pin cache-lock lifecycle.
//!
//! A pin holds one active cache lock per [`crate::pin::PinId`]. The lock
//! is acquired when the pin opens and released exactly once when the pin
//! closes — every close route (context menu, escape, double-click, visible
//! close control, registry teardown) funnels through the same release path
//! so the contract is one lock per pin, one release per pin.
//!
//! The implementation in this file is the in-memory provider used by the
//! synthetic and test paths. The real Windows implementation plugs in
//! behind the same [`pixelgrab_contracts::PinLockProvider`] trait so the
//! wiring is unchanged.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::PinLockProvider;

/// In-memory pin lock provider. Acquire/release both increment reference
/// counts in a single `Mutex<HashMap<...>>` so the test suite can assert
/// the exact counts across repeated open/close cycles.
#[derive(Debug, Clone, Default)]
pub struct InMemoryPinLockProvider {
    state: Arc<Mutex<HashMap<String, usize>>>,
}

impl InMemoryPinLockProvider {
    /// Build a new empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Convenience: in-memory provider wrapped in an `Arc`.
    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

impl PinLockProvider for InMemoryPinLockProvider {
    fn acquire(&self, capture_id: &str) -> bool {
        let mut state = self.state.lock();
        let entry = state.entry(capture_id.to_string()).or_insert(0);
        let was_zero = *entry == 0;
        *entry += 1;
        was_zero
    }

    fn release(&self, capture_id: &str) -> bool {
        let mut state = self.state.lock();
        match state.get_mut(capture_id) {
            Some(count) if *count > 0 => {
                *count -= 1;
                if *count == 0 {
                    state.remove(capture_id);
                }
                true
            }
            _ => false,
        }
    }

    fn active_locks(&self) -> usize {
        let state = self.state.lock();
        state.values().filter(|c| **c > 0).count()
    }
}

/// Production pin lock provider backed by the cache's shared
/// `ActiveLockSet`. Every open pin holds one `LockOwner::Pin` reference
/// on its source entry, so the sweeper and the manual `clear_cache`
/// cannot evict the pinned PNG while the pin window is alive (issue #63).
///
/// The provider keys on `capture_id` (the [`PinSource`] identity) and
/// resolves it to the owning shelf entry through
/// [`crate::cache::Cache::entry_by_capture`]; capture ids are UUIDv4
/// strings unique per committed entry. A capture id that does not match
/// a committed entry acquires nothing — the IPC layer validates the
/// entry before opening a pin, so this only fires for stale payloads.
#[derive(Debug, Clone)]
pub struct CachePinLockProvider {
    cache: crate::cache::Cache,
}

impl CachePinLockProvider {
    /// Build a provider over the given cache store.
    pub fn new(cache: crate::cache::Cache) -> Self {
        Self { cache }
    }
}

impl PinLockProvider for CachePinLockProvider {
    fn acquire(&self, capture_id: &str) -> bool {
        match self.cache.entry_by_capture(capture_id) {
            Some(entry) => self.cache.acquire_pin_guard(&entry.shelf_id).is_ok(),
            None => false,
        }
    }

    fn release(&self, capture_id: &str) -> bool {
        match self.cache.entry_by_capture(capture_id) {
            Some(entry) => self.cache.release_pin_guard(&entry.shelf_id),
            None => false,
        }
    }

    fn active_locks(&self) -> usize {
        self.cache
            .entries()
            .iter()
            .filter(|entry| self.cache.pin_guard_count(&entry.shelf_id) > 0)
            .count()
    }
}

/// RAII cache lock guard. Holds an `Arc` to the lock provider so the lock
/// survives even after the registry is dropped; the guard's `Drop` impl
/// is the sole release path, so the lock is released exactly once.
#[derive(Debug)]
pub struct PinLockGuard {
    provider: Arc<dyn PinLockProvider>,
    capture_id: String,
}

impl PinLockGuard {
    /// Build a new guard. The lock is acquired by the caller via the
    /// provider; the guard assumes ownership of the lock and releases it
    /// on drop.
    pub fn new(provider: Arc<dyn PinLockProvider>, capture_id: impl Into<String>) -> Self {
        Self {
            provider,
            capture_id: capture_id.into(),
        }
    }

    /// Borrow the capture id the guard is holding.
    pub fn capture_id(&self) -> &str {
        &self.capture_id
    }
}

impl Drop for PinLockGuard {
    fn drop(&mut self) {
        self.provider.release(&self.capture_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_release_round_trips() {
        let provider = InMemoryPinLockProvider::new();
        assert!(provider.acquire("capture-1"));
        assert_eq!(provider.active_locks(), 1);
        assert!(provider.release("capture-1"));
        assert_eq!(provider.active_locks(), 0);
    }

    #[test]
    fn repeated_acquire_is_refcounted() {
        let provider = InMemoryPinLockProvider::new();
        assert!(provider.acquire("c"));
        assert!(!provider.acquire("c"));
        assert_eq!(provider.active_locks(), 1);
        // First release: count 2 -> 1, returns true (it released a ref).
        assert!(provider.release("c"));
        // Second release: count 1 -> 0, returns true (it released the last ref).
        assert!(provider.release("c"));
        // Third release: no lock left, returns false.
        assert!(!provider.release("c"));
        assert_eq!(provider.active_locks(), 0);
    }

    #[test]
    fn release_without_acquire_returns_false() {
        let provider = InMemoryPinLockProvider::new();
        assert!(!provider.release("missing"));
    }

    #[test]
    fn guard_releases_on_drop() {
        let provider = InMemoryPinLockProvider::arc();
        // The guard does NOT acquire — the caller is responsible for the
        // acquire. The guard is the sole owner of the release path.
        let was_zero = provider.acquire("c");
        assert!(was_zero);
        {
            let _guard = PinLockGuard::new(provider.clone(), "c");
            assert_eq!(provider.active_locks(), 1);
        }
        assert_eq!(provider.active_locks(), 0);
    }

    #[test]
    fn cache_provider_round_trips_shared_registry_locks() {
        use pixelgrab_contracts::LockOwner;
        use pixelgrab_test_support::fs::IsolatedFilesystem;

        let fs = IsolatedFilesystem::new("pin-cache-provider").expect("fs");
        let cache = crate::cache::Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let committed = cache
            .commit(crate::cache::CacheCommitRequest {
                bounds: pixelgrab_contracts::coordinate::PhysicalBounds::from_xywh(0, 0, 4, 4),
                size: pixelgrab_contracts::coordinate::PhysicalSize::new(4, 4),
                rgba: vec![0u8; 4 * 4 * 4],
                metadata: Default::default(),
                monitor_id: "primary".into(),
            })
            .expect("commit");
        let capture_id = committed.entry.capture_id.clone();
        let shelf_id = committed.entry.shelf_id.clone();

        let provider = CachePinLockProvider::new(cache.clone());
        // Two pins on the same capture each hold one reference.
        assert!(provider.acquire(&capture_id));
        assert!(provider.acquire(&capture_id));
        assert_eq!(
            cache.locks().owners_of(&shelf_id),
            vec![LockOwner::Shelf, LockOwner::Pin]
        );
        assert_eq!(provider.active_locks(), 1);
        assert!(cache.is_protected_from_sweeper(&shelf_id));

        // The first release drops one ref; the entry stays locked.
        assert!(provider.release(&capture_id));
        assert_eq!(cache.pin_guard_count(&shelf_id), 1);
        assert!(provider.release(&capture_id));
        assert_eq!(cache.pin_guard_count(&shelf_id), 0);
        assert!(!cache.is_protected_from_sweeper(&shelf_id));

        // Releasing without a live ref reports false.
        assert!(!provider.release(&capture_id));
    }

    #[test]
    fn cache_provider_unknown_capture_is_a_noop() {
        let provider = CachePinLockProvider::new(crate::cache::Cache::new());
        assert!(!provider.acquire("no-such-capture"));
        assert!(!provider.release("no-such-capture"));
        assert_eq!(provider.active_locks(), 0);
    }
}
