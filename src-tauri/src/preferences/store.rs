//! Persistent shelf preferences store.
//!
//! Tracer 12 introduces user-configurable shelf settings (corner,
//! target monitor, margins, auto-dismiss, lifetime, visible-card
//! count, countdown visibility). The settings are persisted under
//! `%LOCALAPPDATA%\com.pixelgrab.app\shelf-preferences.json` (next to
//! the cache directory, not inside it — see ADR-0007).
//!
//! ## Persistence model
//!
//! - **Crash-safe**: every write is `temp + fsync + rename`. The
//!   rename either replaces the primary file or leaves it untouched,
//!   so an interrupted write cannot corrupt the file.
//! - **Backup**: a `.bak` copy is kept next to the primary file. On
//!   startup the loader prefers the primary file but falls back to
//!   the backup if the primary fails to parse. If both fail the
//!   defaults are used.
//! - **Debounced**: callers mutate the in-memory state immediately;
//!   the disk write is scheduled after a trailing 500 ms debounce so
//!   dragging a slider does not hammer the filesystem. The debounce
//!   is bypassed for `flush_blocking` (used during clean shutdown)
//!   so a process that exits inside the debounce window does not
//!   lose the most recent change.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use pixelgrab_contracts::{PlatformError, PlatformErrorKind, ShelfPreferences};

use super::debouncer::Debouncer;

/// How long the in-memory state may sit dirty before the disk write
/// fires. Trailing debounce: the timer resets on every update so a
/// continuous slider drag results in exactly one disk write at the
/// end.
pub const PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);

/// Filename for the primary settings file.
pub const PRIMARY_FILENAME: &str = "shelf-preferences.json";
/// Filename for the backup copy. The backup is rotated every time the
/// primary file is successfully written so a partial write still has
/// a known-good fallback.
pub const BACKUP_FILENAME: &str = "shelf-preferences.json.bak";

/// In-memory + on-disk shelf preferences store. Cheap to clone: every
/// field is behind an `Arc` or a `Mutex`.
#[derive(Debug, Clone)]
pub struct PreferencesStore {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Path to the directory the primary and backup files live in.
    /// `None` until `set_root` is called.
    root: Mutex<Option<PathBuf>>,
    /// Current authoritative preferences. Cloned on read so callers
    /// get a stable snapshot.
    current: Mutex<ShelfPreferences>,
    /// Most recently persisted preferences. Used by `cancel_pending`
    /// to revert the in-memory state when an in-flight write is
    /// discarded (e.g. the user drags a slider then releases
    /// outside the commit button).
    last_persisted: Mutex<ShelfPreferences>,
    /// Trailing debouncer; `flush_blocking` drains any pending fire
    /// so a clean shutdown cannot lose a debounced write.
    debouncer: Debouncer,
}

impl PreferencesStore {
    /// Build a new store with the default preferences. The root is
    /// unset until `set_root` is called.
    pub fn new() -> Self {
        let defaults = ShelfPreferences::default();
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
    /// current preferences from disk if present.
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
        // Reset the debouncer so an in-flight write from before the
        // reconfigure does not clobber the freshly-loaded preferences.
        self.inner.debouncer.cancel();
        Ok(())
    }

    /// Return the current preferences. Always returns a sanitized
    /// value; out-of-range fields were clamped at load time.
    pub fn current(&self) -> ShelfPreferences {
        self.inner.current.lock().clone()
    }

    /// Replace the current preferences. Updates the in-memory state
    /// immediately and schedules a debounced disk write. Pass `None`
    /// for `on_flush` to skip the notification hook.
    pub fn update(
        &self,
        preferences: ShelfPreferences,
        on_flush: Option<Arc<dyn Fn() + Send + Sync>>,
    ) {
        let sanitized = preferences.sanitize();
        *self.inner.current.lock() = sanitized.clone();
        let root = self.inner.root.lock().clone();
        let Some(root) = root else {
            // Without a root there's nothing to persist. The in-memory
            // state is still updated so callers see the new value.
            return;
        };
        let store = self.clone();
        self.inner.debouncer.schedule(move || {
            match write_to_disk(&root, &sanitized) {
                Ok(()) => {
                    // Track the most recently persisted snapshot so
                    // `cancel_pending` can revert to it.
                    *store.inner.last_persisted.lock() = sanitized.clone();
                    if let Some(cb) = on_flush.as_ref() {
                        cb();
                    }
                }
                Err(err) => {
                    // Privacy: log the categorical kind, not the
                    // error message (which can include the path).
                    log::warn!("preferences debounced flush failed: {:?}", err.kind);
                }
            }
        });
    }

    /// Force an immediate disk write of the current preferences.
    /// Returns the error from the failed write so callers can decide
    /// how to surface it; never blocks on the debouncer's timer.
    pub fn flush_blocking(&self) -> Result<(), PlatformError> {
        self.inner.debouncer.cancel();
        let root = self.inner.root.lock().clone();
        let Some(root) = root else {
            return Ok(());
        };
        let prefs = self.inner.current.lock().clone();
        write_to_disk(&root, &prefs)?;
        *self.inner.last_persisted.lock() = prefs;
        Ok(())
    }

    /// Cancel any pending debounced write **and** revert the
    /// in-memory state to the most recently persisted snapshot.
    /// This makes "cancel" mean "abort the change entirely" — the
    /// same semantic the user expects when releasing a slider
    /// outside the commit button.
    pub fn cancel_pending(&self) {
        self.inner.debouncer.cancel();
        let last = self.inner.last_persisted.lock().clone();
        *self.inner.current.lock() = last;
    }

    /// Test-only: simulate that the debounce window elapsed by
    /// running the pending callback synchronously. Returns `true` if
    /// a callback ran.
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

    /// Test-only: return the backup file path, if the root is
    /// configured.
    pub fn backup_path(&self) -> Option<PathBuf> {
        self.inner
            .root
            .lock()
            .as_ref()
            .map(|r| r.join(BACKUP_FILENAME))
    }
}

impl Default for PreferencesStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Read preferences from disk. Prefers the primary file, falls back to
/// the backup, and finally to the defaults. Invalid files (parse
/// errors, schema mismatches, out-of-range numbers) are recovered by
/// returning the defaults — a corrupt settings file must not crash
/// the app.
pub(crate) fn load_from_disk(root: &Path) -> Option<ShelfPreferences> {
    let primary = root.join(PRIMARY_FILENAME);
    if let Some(prefs) = try_load(&primary) {
        return Some(prefs);
    }
    let backup = root.join(BACKUP_FILENAME);
    if let Some(prefs) = try_load(&backup) {
        return Some(prefs);
    }
    None
}

fn try_load(path: &Path) -> Option<ShelfPreferences> {
    let bytes = fs::read(path).ok()?;
    let parsed: ShelfPreferences = serde_json::from_slice(&bytes).ok()?;
    Some(parsed.sanitize())
}

/// Write preferences to disk atomically with a backup. The primary
/// file is written to a sibling `*.tmp`, fsync'd, then renamed into
/// place. If a previous primary file existed, it is rotated into the
/// `.bak` slot before the rename so the previous good state is
/// preserved.
pub(crate) fn write_to_disk(
    root: &Path,
    preferences: &ShelfPreferences,
) -> Result<(), PlatformError> {
    let primary = root.join(PRIMARY_FILENAME);
    let backup = root.join(BACKUP_FILENAME);
    let tmp = root.join(format!("{PRIMARY_FILENAME}.tmp"));

    let bytes = serde_json::to_vec_pretty(preferences).map_err(|err| {
        PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!("serialise preferences: {err}"),
        )
    })?;

    // Rotate the previous primary to the backup slot so the previous
    // good state is preserved across an interrupted write. Skipped
    // when the primary file is absent (first run).
    if primary.exists() {
        // On Windows `rename` over an existing destination fails; the
        // backup slot must be removed first. Use a best-effort
        // remove that swallows the not-found error.
        if backup.exists() {
            let _ = fs::remove_file(&backup);
        }
        fs::rename(&primary, &backup).map_err(|err| {
            PlatformError::new(
                PlatformErrorKind::Io,
                format!("rotate primary to backup: {err}"),
            )
        })?;
    }

    // Write the new primary atomically: tmp + fsync + rename.
    let mut file = fs::File::create(&tmp).map_err(|err| {
        PlatformError::new(
            PlatformErrorKind::Io,
            format!("create tmp {}: {err}", tmp.display()),
        )
    })?;
    file.write_all(&bytes).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        PlatformError::new(
            PlatformErrorKind::Io,
            format!("write tmp {}: {err}", tmp.display()),
        )
    })?;
    file.sync_all().map_err(|err| {
        let _ = fs::remove_file(&tmp);
        PlatformError::new(
            PlatformErrorKind::Io,
            format!("fsync tmp {}: {err}", tmp.display()),
        )
    })?;
    drop(file);

    if let Err(err) = fs::rename(&tmp, &primary) {
        let _ = fs::remove_file(&tmp);
        return Err(PlatformError::new(
            PlatformErrorKind::Io,
            format!("rename tmp into primary: {err}"),
        ));
    }

    // fsync the directory so the new primary survives a power loss.
    if let Some(parent) = primary.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

/// Convenience: resolve the on-disk root directory for preferences on
/// the current platform. Windows uses
/// `%LOCALAPPDATA%\com.pixelgrab.app`; non-Windows (CI, dev) falls
/// back to the system temp directory so tests have a stable writable
/// home.
pub fn default_preferences_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local_app_data).join("com.pixelgrab.app");
        }
    }
    std::env::temp_dir().join("com.pixelgrab.app")
}

#[cfg(test)]
mod tests {
    use super::*;

    use pixelgrab_test_support::fs::IsolatedFilesystem;

    #[test]
    fn default_state_is_in_memory_only() {
        let store = PreferencesStore::new();
        let current = store.current();
        assert_eq!(current, ShelfPreferences::default());
        assert!(store.primary_path().is_none());
    }

    #[test]
    fn set_root_loads_existing_preferences() {
        let fs = IsolatedFilesystem::new("prefs-root-load").expect("fs");
        let written = ShelfPreferences {
            lifetime_seconds: 90,
            ..ShelfPreferences::default()
        };
        write_to_disk(fs.root(), &written).expect("seed");

        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(store.current().lifetime_seconds, 90);
    }

    #[test]
    fn set_root_recovers_from_corrupt_primary() {
        let fs = IsolatedFilesystem::new("prefs-corrupt-primary").expect("fs");
        // Plant a primary file that does not parse, plus a valid backup.
        std::fs::write(fs.join(PRIMARY_FILENAME), b"not valid json").expect("write");
        let backup = ShelfPreferences {
            lifetime_seconds: 120,
            ..ShelfPreferences::default()
        };
        write_to_disk_for_backup(fs.root(), BACKUP_FILENAME, &backup);

        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");
        // The primary file failed to parse, the backup loaded.
        assert_eq!(store.current().lifetime_seconds, 120);
    }

    #[test]
    fn set_root_falls_back_to_defaults_when_both_corrupt() {
        let fs = IsolatedFilesystem::new("prefs-both-corrupt").expect("fs");
        std::fs::write(fs.join(PRIMARY_FILENAME), b"garbage").expect("write primary");
        std::fs::write(fs.join(BACKUP_FILENAME), b"also garbage").expect("write backup");

        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(store.current(), ShelfPreferences::default());
    }

    #[test]
    fn update_writes_through_atomically() {
        let fs = IsolatedFilesystem::new("prefs-write").expect("fs");
        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let next = ShelfPreferences {
            corner: pixelgrab_contracts::ShelfCorner::TopLeft,
            lifetime_seconds: 45,
            ..ShelfPreferences::default()
        };
        store.update(next.clone(), None);
        store.flush_blocking().expect("flush");

        let reloaded = PreferencesStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        assert_eq!(
            reloaded.current().corner,
            pixelgrab_contracts::ShelfCorner::TopLeft
        );
        assert_eq!(reloaded.current().lifetime_seconds, 45);
    }

    #[test]
    fn update_sanitizes_before_writing() {
        let fs = IsolatedFilesystem::new("prefs-sanitize").expect("fs");
        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let dirty = ShelfPreferences {
            margin_px: 9_999,
            lifetime_seconds: 1_000,
            visible_card_count: 99,
            ..ShelfPreferences::default()
        };
        store.update(dirty, None);
        store.flush_blocking().expect("flush");

        let reloaded = PreferencesStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        assert_eq!(
            reloaded.current().margin_px,
            pixelgrab_contracts::MAX_MARGIN_PX
        );
        assert_eq!(
            reloaded.current().lifetime_seconds,
            pixelgrab_contracts::MAX_LIFETIME_SECONDS
        );
        // visible_card_count is still in the preference module; clamp
        // to its own ceiling rather than the queue's MAX_VISIBLE_CARDS.
        assert!(reloaded.current().visible_card_count <= 8);
    }

    #[test]
    fn flush_blocking_drains_pending_debounce() {
        let fs = IsolatedFilesystem::new("prefs-flush-drain").expect("fs");
        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let next = ShelfPreferences {
            corner: pixelgrab_contracts::ShelfCorner::BottomLeft,
            ..ShelfPreferences::default()
        };
        store.update(next.clone(), None);
        // Without flush_blocking the disk has not been touched yet.
        assert!(!fs.join(PRIMARY_FILENAME).exists());

        store.flush_blocking().expect("flush");
        assert!(fs.join(PRIMARY_FILENAME).exists());
    }

    #[test]
    fn write_creates_backup_on_rotation() {
        let fs = IsolatedFilesystem::new("prefs-backup").expect("fs");
        // Seed the primary with one set of values.
        let first = ShelfPreferences {
            lifetime_seconds: 30,
            ..ShelfPreferences::default()
        };
        write_to_disk(fs.root(), &first).expect("first write");
        // A second write rotates the previous primary to backup.
        let second = ShelfPreferences {
            lifetime_seconds: 60,
            ..ShelfPreferences::default()
        };
        write_to_disk(fs.root(), &second).expect("second write");

        // Primary holds the most recent value.
        let primary = fs::read_to_string(fs.join(PRIMARY_FILENAME)).expect("primary");
        assert!(primary.contains("\"lifetimeSeconds\": 60"));
        // Backup holds the previous value.
        let backup = fs::read_to_string(fs.join(BACKUP_FILENAME)).expect("backup");
        assert!(backup.contains("\"lifetimeSeconds\": 30"));
    }

    #[test]
    fn cancel_pending_discards_in_flight_write() {
        let fs = IsolatedFilesystem::new("prefs-cancel").expect("fs");
        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let next = ShelfPreferences {
            lifetime_seconds: 15,
            ..ShelfPreferences::default()
        };
        store.update(next, None);
        store.cancel_pending();
        store.flush_blocking().expect("flush");

        // The cancelled update was not written; the file holds defaults.
        let primary = fs::read_to_string(fs.join(PRIMARY_FILENAME)).expect("primary");
        assert!(primary.contains("\"lifetimeSeconds\": 60"));
    }

    #[test]
    fn drain_pending_for_test_runs_callback_immediately() {
        let fs = IsolatedFilesystem::new("prefs-drain").expect("fs");
        let store = PreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let next = ShelfPreferences {
            lifetime_seconds: 7,
            ..ShelfPreferences::default()
        };
        store.update(next, None);

        let ran = store.drain_pending_for_test();
        assert!(ran);
        assert!(fs.join(PRIMARY_FILENAME).exists());
    }

    #[test]
    fn update_without_root_keeps_in_memory_state() {
        let store = PreferencesStore::new();
        let next = ShelfPreferences {
            lifetime_seconds: 20,
            ..ShelfPreferences::default()
        };
        store.update(next.clone(), None);
        // No root configured, so no panic; in-memory state is updated.
        assert_eq!(store.current().lifetime_seconds, 20);
    }

    #[test]
    fn default_preferences_root_is_writable_or_temp() {
        let path = default_preferences_root();
        // Smoke: the path must be absolute. Specific platform paths
        // are exercised in integration tests on the target OS.
        assert!(path.is_absolute());
    }

    /// Helper: write a specific filename as the primary would write
    /// it. Used to seed the backup slot independently of the primary.
    fn write_to_disk_for_backup(root: &Path, filename: &str, prefs: &ShelfPreferences) {
        let target = root.join(filename);
        let bytes = serde_json::to_vec_pretty(prefs).expect("serialise");
        std::fs::write(&target, &bytes).expect("write");
    }
}
