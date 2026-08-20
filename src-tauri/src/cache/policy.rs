//! Persistent cache policy store.
//!
//! Tracer 13 introduces user-configurable cache bounds
//! (max bytes / max entries / max age / low-water ratio / sweep
//! interval / purge-on-exit). The settings are persisted under
//! `%LOCALAPPDATA%\com.pixelgrab.app\cache-policy.json` (next to the
//! shelf-preferences file, outside the cache root so a partial cache
//! reap cannot delete the user's policy).
//!
//! ## Persistence model
//!
//! Mirrors the shelf preferences store: every write is
//! `temp + fsync + rename` with a `.bak` rotation so the previous
//! good state survives an interrupted write. Writes are debounced
//! with a 500 ms trailing window so the settings UI does not hammer
//! the filesystem; a `flush_blocking` helper bypasses the debounce
//! for clean shutdown.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::{CachePolicy, PlatformError, PlatformErrorKind};

use crate::preferences::debouncer::Debouncer;

/// How long the in-memory state may sit dirty before the disk write
/// fires. Trailing debounce — the timer resets on every update so a
/// continuous slider drag results in exactly one disk write at the
/// end.
pub const PERSIST_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Filename for the primary policy file.
pub const PRIMARY_FILENAME: &str = "cache-policy.json";
/// Filename for the backup copy. The backup is rotated every time the
/// primary file is successfully written so a partial write still has
/// a known-good fallback.
pub const BACKUP_FILENAME: &str = "cache-policy.json.bak";

/// In-memory + on-disk cache policy store. Cheap to clone: every
/// field is behind an `Arc` or a `Mutex`.
#[derive(Debug, Clone)]
pub struct CachePolicyStore {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Path to the directory the primary and backup files live in.
    /// `None` until `set_root` is called.
    root: Mutex<Option<PathBuf>>,
    /// Current authoritative policy. Cloned on read so callers get a
    /// stable snapshot.
    current: Mutex<CachePolicy>,
    /// Most recently persisted policy. Used by `cancel_pending` to
    /// revert the in-memory state when an in-flight write is
    /// discarded (e.g. the user drags a slider then releases
    /// outside the commit button).
    last_persisted: Mutex<CachePolicy>,
    /// Trailing debouncer; `flush_blocking` drains any pending fire
    /// so a clean shutdown cannot lose a debounced write.
    debouncer: Debouncer,
}

impl CachePolicyStore {
    /// Build a new store with the default policy. The root is unset
    /// until `set_root` is called.
    pub fn new() -> Self {
        let defaults = CachePolicy::default();
        Self {
            inner: Arc::new(Inner {
                root: Mutex::new(None),
                current: Mutex::new(defaults.clone()),
                last_persisted: Mutex::new(defaults),
                debouncer: Debouncer::new(PERSIST_DEBOUNCE),
            }),
        }
    }

    /// Configure the on-disk root directory. Idempotent. Loads the
    /// current policy from disk if present.
    pub fn set_root(&self, root: PathBuf) -> Result<(), PlatformError> {
        if !root.as_os_str().is_empty() {
            fs::create_dir_all(&root).map_err(|err| {
                PlatformError::new(
                    PlatformErrorKind::Io,
                    format!("create_dir_all({}): {err}", root.display()),
                )
            })?;
        }
        let loaded = load_from_disk(&root).unwrap_or_default();
        *self.inner.current.lock() = loaded.clone();
        *self.inner.last_persisted.lock() = loaded;
        *self.inner.root.lock() = Some(root);
        self.inner.debouncer.cancel();
        Ok(())
    }

    /// Return the current policy. Always returns a sanitized value;
    /// out-of-range fields were clamped at load time.
    pub fn current(&self) -> CachePolicy {
        self.inner.current.lock().clone()
    }

    /// Replace the current policy. Updates the in-memory state
    /// immediately and schedules a debounced disk write.
    pub fn update(&self, policy: CachePolicy) {
        let sanitized = policy.sanitize();
        *self.inner.current.lock() = sanitized.clone();
        let root = self.inner.root.lock().clone();
        let Some(root) = root else {
            // Without a root there's nothing to persist. The
            // in-memory state is still updated so callers see the
            // new value.
            return;
        };
        let store = self.clone();
        self.inner.debouncer.schedule(move || {
            match write_to_disk(&root, &sanitized) {
                Ok(()) => {
                    *store.inner.last_persisted.lock() = sanitized;
                }
                Err(_err) => {
                    // Privacy: log the categorical kind, not the
                    // error message (which can include the path).
                    log::warn!("cache policy debounced flush failed");
                }
            }
        });
    }

    /// Force an immediate disk write of the current policy. Returns
    /// the error from the failed write so callers can decide how to
    /// surface it; never blocks on the debouncer's timer.
    pub fn flush_blocking(&self) -> Result<(), PlatformError> {
        self.inner.debouncer.cancel();
        let root = self.inner.root.lock().clone();
        let Some(root) = root else {
            return Ok(());
        };
        let policy = self.inner.current.lock().clone();
        write_to_disk(&root, &policy)?;
        *self.inner.last_persisted.lock() = policy;
        Ok(())
    }

    /// Test-only: simulate that the debounce window elapsed by
    /// running the pending callback synchronously.
    pub fn drain_pending_for_test(&self) -> bool {
        self.inner.debouncer.drain_for_test()
    }

    /// Test-only: return the path the primary file would be written
    /// to, if the root is configured.
    pub fn primary_path(&self) -> Option<PathBuf> {
        self.inner
            .root
            .lock()
            .as_ref()
            .map(|r| r.join(PRIMARY_FILENAME))
    }
}

impl Default for CachePolicyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Read policy from disk. Prefers the primary file, falls back to the
/// backup, and finally to the defaults. Invalid files (parse errors,
/// schema mismatches, out-of-range numbers) are recovered by returning
/// the defaults — a corrupt settings file must not crash the app.
pub(crate) fn load_from_disk(root: &Path) -> Option<CachePolicy> {
    let primary = root.join(PRIMARY_FILENAME);
    if let Some(policy) = try_load(&primary) {
        return Some(policy);
    }
    let backup = root.join(BACKUP_FILENAME);
    if let Some(policy) = try_load(&backup) {
        return Some(policy);
    }
    None
}

fn try_load(path: &Path) -> Option<CachePolicy> {
    let bytes = fs::read(path).ok()?;
    let parsed: CachePolicy = serde_json::from_slice(&bytes).ok()?;
    Some(parsed.sanitize())
}

/// Write policy to disk atomically with a backup. Mirrors the shelf
/// preferences writer: temp + fsync + rename, with the previous
/// primary rotated into the `.bak` slot so an interrupted write still
/// has a known-good fallback.
pub(crate) fn write_to_disk(root: &Path, policy: &CachePolicy) -> Result<(), PlatformError> {
    let primary = root.join(PRIMARY_FILENAME);
    let backup = root.join(BACKUP_FILENAME);
    let tmp = root.join(format!("{PRIMARY_FILENAME}.tmp"));

    let bytes = serde_json::to_vec_pretty(policy).map_err(|err| {
        PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!("serialise policy: {err}"),
        )
    })?;

    if primary.exists() {
        if backup.exists() {
            let _ = fs::remove_file(&backup);
        }
        fs::rename(&primary, &backup).map_err(|_err| {
            // Privacy: never interpolate the path into the error
            // string — the IPC payload is the wire shape (AGENTS.md §9).
            PlatformError::new(PlatformErrorKind::Io, "rotate primary to backup")
        })?;
    }

    let mut file = fs::File::create(&tmp)
        .map_err(|_err| PlatformError::new(PlatformErrorKind::Io, "create policy tmp"))?;
    file.write_all(&bytes).map_err(|_err| {
        let _ = fs::remove_file(&tmp);
        PlatformError::new(PlatformErrorKind::Io, "write policy tmp")
    })?;
    file.sync_all().map_err(|_err| {
        let _ = fs::remove_file(&tmp);
        PlatformError::new(PlatformErrorKind::Io, "fsync policy tmp")
    })?;
    drop(file);

    if let Err(_err) = fs::rename(&tmp, &primary) {
        let _ = fs::remove_file(&tmp);
        return Err(PlatformError::new(
            PlatformErrorKind::Io,
            "rename policy tmp into primary",
        ));
    }

    if let Some(parent) = primary.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use pixelgrab_test_support::fs::IsolatedFilesystem;

    #[test]
    fn default_state_is_in_memory_only() {
        let store = CachePolicyStore::new();
        let current = store.current();
        assert_eq!(current, CachePolicy::default());
        assert!(store.primary_path().is_none());
    }

    #[test]
    fn set_root_loads_existing_policy() {
        let fs = IsolatedFilesystem::new("cache-policy-load").expect("fs");
        let written = CachePolicy {
            max_bytes: 1024 * 1024,
            max_entries: 25,
            ..CachePolicy::default()
        };
        write_to_disk(fs.root(), &written).expect("seed");

        let store = CachePolicyStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(store.current().max_bytes, 1024 * 1024);
        assert_eq!(store.current().max_entries, 25);
    }

    #[test]
    fn set_root_recovers_from_corrupt_primary() {
        let fs = IsolatedFilesystem::new("cache-policy-corrupt").expect("fs");
        std::fs::write(fs.join(PRIMARY_FILENAME), b"not valid json").expect("write");
        let backup = CachePolicy {
            max_bytes: 5 * 1024 * 1024,
            ..CachePolicy::default()
        };
        std::fs::write(
            fs.join(BACKUP_FILENAME),
            serde_json::to_vec_pretty(&backup).expect("serialise"),
        )
        .expect("write backup");

        let store = CachePolicyStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(store.current().max_bytes, 5 * 1024 * 1024);
    }

    #[test]
    fn set_root_falls_back_to_defaults_when_both_corrupt() {
        let fs = IsolatedFilesystem::new("cache-policy-both-corrupt").expect("fs");
        std::fs::write(fs.join(PRIMARY_FILENAME), b"garbage").expect("write primary");
        std::fs::write(fs.join(BACKUP_FILENAME), b"also garbage").expect("write backup");

        let store = CachePolicyStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(store.current(), CachePolicy::default());
    }

    #[test]
    fn update_writes_through_atomically() {
        let fs = IsolatedFilesystem::new("cache-policy-write").expect("fs");
        let store = CachePolicyStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let next = CachePolicy {
            max_bytes: 99 * 1024 * 1024,
            max_entries: 99,
            ..CachePolicy::default()
        };
        store.update(next.clone());
        store.flush_blocking().expect("flush");

        let reloaded = CachePolicyStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        assert_eq!(reloaded.current().max_bytes, 99 * 1024 * 1024);
        assert_eq!(reloaded.current().max_entries, 99);
    }

    #[test]
    fn update_sanitizes_before_writing() {
        let fs = IsolatedFilesystem::new("cache-policy-sanitize").expect("fs");
        let store = CachePolicyStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let dirty = CachePolicy {
            max_bytes: 10,
            max_entries: 0,
            max_age_ms: 5,
            low_water_ratio: 2.0,
            sweep_interval_ms: 5,
            purge_on_exit: true,
            ..CachePolicy::default()
        };
        store.update(dirty);
        store.flush_blocking().expect("flush");

        let reloaded = CachePolicyStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        let p = reloaded.current();
        assert_eq!(p.max_bytes, pixelgrab_contracts::MIN_MAX_BYTES);
        assert_eq!(p.max_entries, pixelgrab_contracts::MIN_MAX_ENTRIES);
        assert_eq!(p.max_age_ms, pixelgrab_contracts::MIN_MAX_AGE_MS);
        assert_eq!(p.low_water_ratio, pixelgrab_contracts::MAX_LOW_WATER_RATIO);
        assert_eq!(
            p.sweep_interval_ms,
            pixelgrab_contracts::MIN_SWEEP_INTERVAL_MS
        );
        assert!(p.purge_on_exit);
    }

    #[test]
    fn flush_after_update_persists_new_value() {
        let fs = IsolatedFilesystem::new("cache-policy-cancel").expect("fs");
        let store = CachePolicyStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");
        store.flush_blocking().expect("seed");

        let next = CachePolicy {
            max_bytes: 7 * 1024 * 1024,
            ..CachePolicy::default()
        };
        store.update(next);
        store.flush_blocking().expect("flush");

        let primary = std::fs::read_to_string(fs.join(PRIMARY_FILENAME)).expect("primary");
        // The flush_blocking after update persisted the new
        // value — the "cancel" path is no longer exposed.
        assert!(primary.contains(&format!("\"maxBytes\": {}", 7 * 1024 * 1024)));
    }

    #[test]
    fn update_without_root_keeps_in_memory_state() {
        let store = CachePolicyStore::new();
        let next = CachePolicy {
            max_bytes: 11 * 1024 * 1024,
            ..CachePolicy::default()
        };
        store.update(next.clone());
        assert_eq!(store.current().max_bytes, 11 * 1024 * 1024);
    }

    #[test]
    fn drain_pending_for_test_runs_callback_immediately() {
        let fs = IsolatedFilesystem::new("cache-policy-drain").expect("fs");
        let store = CachePolicyStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let next = CachePolicy {
            max_bytes: 9 * 1024 * 1024,
            ..CachePolicy::default()
        };
        store.update(next);
        let ran = store.drain_pending_for_test();
        assert!(ran);
        assert!(fs.join(PRIMARY_FILENAME).exists());
    }
}
