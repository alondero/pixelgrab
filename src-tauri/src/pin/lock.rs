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
}
