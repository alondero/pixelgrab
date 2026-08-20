//! Persistent hotkey bindings store.
//!
//! Tracer 14 lifts the registered shortcuts out of the tray-build
//! hard-coding and into a JSON document next to
//! `shelf-preferences.json`. The store mirrors the shape used by
//! the shelf preferences — atomic write with backup rotation,
//! default fallback on parse failure — but skips the debouncer
//! because each rebind is meant to commit immediately. The IPC
//! layer returns synchronously only after the on-disk write has
//! returned, so a hotkey change cannot be "lost" to a debounce
//! window.
//!
//! ## Persistence model
//!
//! - **Crash-safe**: every write is `temp + fsync + rename`. The
//!   rename either replaces the primary file or leaves it
//!   untouched, so an interrupted write cannot corrupt the file.
//! - **Backup**: a `.bak` copy is kept next to the primary file.
//!   On startup the loader prefers the primary file but falls back
//!   to the backup if the primary fails to parse. If both fail
//!   the defaults are used.
//! - **No debounce**: callers replace the in-memory state and
//!   trigger a synchronous atomic write via `update`. The IPC
//!   layer calls `flush_blocking` during shutdown so a process
//!   that exits immediately after the last write still finds the
//!   disk in sync.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::{
    HotkeyAction, HotkeyBindings, PlatformError, PlatformErrorKind, SanitizeReport,
    HOTKEY_BACKUP_FILENAME, HOTKEY_PRIMARY_FILENAME,
};

/// In-memory + on-disk hotkey bindings store. Cheap to clone:
/// every field is behind an `Arc` or a `Mutex`.
#[derive(Debug, Clone)]
pub struct HotkeyPreferencesStore {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Path to the directory the primary and backup files live in.
    /// `None` until `set_root` is called.
    root: Mutex<Option<PathBuf>>,
    /// Current authoritative bindings. Cloned on read so callers
    /// get a stable snapshot.
    current: Mutex<HotkeyBindings>,
}

impl HotkeyPreferencesStore {
    /// Build a new store with the default bindings. The root is
    /// unset until `set_root` is called.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                root: Mutex::new(None),
                current: Mutex::new(HotkeyBindings::defaults()),
            }),
        }
    }

    /// Configure the on-disk root directory. Idempotent. Loads the
    /// current bindings from disk if present, sanitising on the way
    /// in.
    pub fn set_root(&self, root: PathBuf) -> Result<SanitizeReport, PlatformError> {
        if !root.as_os_str().is_empty() {
            fs::create_dir_all(&root).map_err(|err| {
                PlatformError::new(
                    PlatformErrorKind::Io,
                    format!("create_dir_all({}): {err}", root.display()),
                )
            })?;
        }
        let loaded = load_from_disk(&root).unwrap_or_default();
        let (sanitized, report) = loaded.sanitize();
        *self.inner.current.lock() = sanitized;
        *self.inner.root.lock() = Some(root);
        Ok(report)
    }

    /// Return the current bindings snapshot.
    pub fn current(&self) -> HotkeyBindings {
        self.inner.current.lock().clone()
    }

    /// Replace the current bindings. The in-memory state is
    /// updated immediately; an atomic disk write is scheduled
    /// synchronously and the call surfaces any IO failure.
    pub fn update(&self, bindings: HotkeyBindings) -> Result<(), PlatformError> {
        let (sanitized, _report) = bindings.sanitize();
        *self.inner.current.lock() = sanitized.clone();
        let root = self.inner.root.lock().clone();
        let Some(root) = root else {
            // Without a root there's nothing to persist. The
            // in-memory state is still updated so callers see the
            // new value.
            return Ok(());
        };
        write_to_disk(&root, &sanitized)
    }

    /// Replace a single binding without touching the others. The
    /// change is atomic; passing `binding = None` unbinds the
    /// action.
    pub fn set_binding(
        &self,
        action: HotkeyAction,
        binding: Option<String>,
    ) -> Result<bool, PlatformError> {
        let mut current = self.inner.current.lock().clone();
        let changed = current.set(action, binding);
        if !changed {
            return Ok(false);
        }
        self.update(current)?;
        Ok(true)
    }

    /// Set the paused flag and persist.
    pub fn set_paused(&self, paused: bool) -> Result<bool, PlatformError> {
        let mut current = self.inner.current.lock().clone();
        let changed = current.set_paused(paused);
        if !changed {
            return Ok(false);
        }
        self.update(current)?;
        Ok(true)
    }

    /// Force an immediate disk write of the current bindings.
    /// Returns the error from the failed write so callers can
    /// decide how to surface it; never blocks on the debouncer.
    pub fn flush_blocking(&self) -> Result<(), PlatformError> {
        let root = self.inner.root.lock().clone();
        let Some(root) = root else {
            return Ok(());
        };
        let bindings = self.inner.current.lock().clone();
        write_to_disk(&root, &bindings)
    }

    /// Test-only: return the path the primary file would be
    /// written to, if the root is configured.
    pub fn primary_path(&self) -> Option<PathBuf> {
        self.inner
            .root
            .lock()
            .as_ref()
            .map(|r| r.join(HOTKEY_PRIMARY_FILENAME))
    }

    /// Test-only: return the backup file path, if the root is
    /// configured.
    pub fn backup_path(&self) -> Option<PathBuf> {
        self.inner
            .root
            .lock()
            .as_ref()
            .map(|r| r.join(HOTKEY_BACKUP_FILENAME))
    }
}

impl Default for HotkeyPreferencesStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Read bindings from disk. Prefers the primary file, falls back
/// to the backup, and finally to the defaults. Invalid files
/// (parse errors, schema mismatches, out-of-range numbers) are
/// recovered by returning the defaults — a corrupt settings file
/// must not crash the app.
pub(crate) fn load_from_disk(root: &Path) -> Option<HotkeyBindings> {
    let primary = root.join(HOTKEY_PRIMARY_FILENAME);
    if let Some(bindings) = try_load(&primary) {
        return Some(bindings);
    }
    let backup = root.join(HOTKEY_BACKUP_FILENAME);
    if let Some(bindings) = try_load(&backup) {
        return Some(bindings);
    }
    None
}

fn try_load(path: &Path) -> Option<HotkeyBindings> {
    let bytes = fs::read(path).ok()?;
    let parsed: HotkeyBindings = match serde_json::from_slice(&bytes) {
        Ok(b) => b,
        Err(err) => {
            // Privacy: log the parse failure categorically — the
            // error message can include file paths.
            log::warn!(
                "{:?} failed to parse: {:?} (kind={:?})",
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<bindings>"),
                err.classify(),
                err
            );
            return None;
        }
    };
    Some(parsed)
}

/// Write bindings to disk atomically with a backup. The primary
/// file is written to a sibling `*.tmp`, fsync'd, then renamed
/// into place. If a previous primary file existed, it is rotated
/// into the `.bak` slot before the rename so the previous good
/// state is preserved.
fn write_to_disk(root: &Path, bindings: &HotkeyBindings) -> Result<(), PlatformError> {
    let primary = root.join(HOTKEY_PRIMARY_FILENAME);
    let backup = root.join(HOTKEY_BACKUP_FILENAME);
    let tmp = root.join(format!("{HOTKEY_PRIMARY_FILENAME}.tmp"));

    let bytes = serde_json::to_vec_pretty(bindings).map_err(|err| {
        PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!("serialise hotkey bindings: {err}"),
        )
    })?;

    if primary.exists() {
        // On Windows `rename` over an existing destination fails;
        // the backup slot must be removed first. Use a best-effort
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

#[cfg(test)]
mod tests {
    use super::*;

    use pixelgrab_contracts::HOTKEY_SETTINGS_SCHEMA_VERSION;
    use pixelgrab_test_support::fs::IsolatedFilesystem;

    #[test]
    fn default_state_is_in_memory_only() {
        let store = HotkeyPreferencesStore::new();
        assert_eq!(store.current(), HotkeyBindings::defaults());
        assert!(store.primary_path().is_none());
    }

    #[test]
    fn set_root_loads_existing_bindings() {
        let fs = IsolatedFilesystem::new("hotkey-prefs-load").expect("fs");
        let mut written = HotkeyBindings::defaults();
        written.region_capture = Some("CommandOrControl+Alt+R".to_string());
        write_to_disk_for_seed(fs.root(), HOTKEY_PRIMARY_FILENAME, &written);

        let store = HotkeyPreferencesStore::new();
        let _report = store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(
            store.current().region_capture.as_deref(),
            Some("CommandOrControl+Alt+R")
        );
    }

    #[test]
    fn set_root_recovers_from_corrupt_primary() {
        let fs = IsolatedFilesystem::new("hotkey-corrupt-primary").expect("fs");
        std::fs::write(fs.join(HOTKEY_PRIMARY_FILENAME), b"not json").expect("primary");
        let mut backup = HotkeyBindings::defaults();
        backup.shelf_toggle = Some("CommandOrControl+Alt+L".to_string());
        write_to_disk_for_seed(fs.root(), HOTKEY_BACKUP_FILENAME, &backup);

        let store = HotkeyPreferencesStore::new();
        let _report = store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(
            store.current().shelf_toggle.as_deref(),
            Some("CommandOrControl+Alt+L")
        );
    }

    #[test]
    fn set_root_falls_back_to_defaults_when_both_corrupt() {
        let fs = IsolatedFilesystem::new("hotkey-both-corrupt").expect("fs");
        std::fs::write(fs.join(HOTKEY_PRIMARY_FILENAME), b"garbage").expect("primary");
        std::fs::write(fs.join(HOTKEY_BACKUP_FILENAME), b"also garbage").expect("backup");

        let store = HotkeyPreferencesStore::new();
        let _report = store.set_root(fs.root().to_path_buf()).expect("set root");
        assert_eq!(store.current(), HotkeyBindings::defaults());
    }

    #[test]
    fn update_writes_through_atomically() {
        let fs = IsolatedFilesystem::new("hotkey-write").expect("fs");
        let store = HotkeyPreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let mut next = HotkeyBindings::defaults();
        next.region_capture = Some("CommandOrControl+Alt+R".to_string());
        store.update(next.clone()).expect("update");

        // Re-read from disk to confirm atomic write succeeded.
        let reloaded = HotkeyPreferencesStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        assert_eq!(
            reloaded.current().region_capture.as_deref(),
            Some("CommandOrControl+Alt+R")
        );
    }

    #[test]
    fn update_sanitizes_before_writing() {
        let fs = IsolatedFilesystem::new("hotkey-sanitize").expect("fs");
        let store = HotkeyPreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let dirty = HotkeyBindings {
            schema_version: 0,
            region_capture: Some("Bogus+S".to_string()),
            ..HotkeyBindings::defaults()
        };
        store.update(dirty).expect("update");

        let reloaded = HotkeyPreferencesStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        // Malformed bindings dropped to None on disk.
        assert!(reloaded.current().region_capture.is_none());
        assert_eq!(
            reloaded.current().schema_version,
            HOTKEY_SETTINGS_SCHEMA_VERSION
        );
    }

    #[test]
    fn set_binding_persists_atomically() {
        let fs = IsolatedFilesystem::new("hotkey-set-binding").expect("fs");
        let store = HotkeyPreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let changed = store
            .set_binding(HotkeyAction::ShelfToggle, Some("Ctrl+Alt+L".to_string()))
            .expect("set_binding");
        assert!(changed);
        let reloaded = HotkeyPreferencesStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        assert_eq!(
            reloaded.current().shelf_toggle.as_deref(),
            Some("CommandOrControl+Alt+L")
        );
    }

    #[test]
    fn set_binding_to_none_unbinds() {
        let fs = IsolatedFilesystem::new("hotkey-unbind").expect("fs");
        let store = HotkeyPreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let changed = store
            .set_binding(HotkeyAction::ShelfToggle, None)
            .expect("set_binding");
        assert!(changed);
        assert!(store.current().shelf_toggle.is_none());
    }

    #[test]
    fn set_binding_to_same_value_reports_no_change() {
        let store = HotkeyPreferencesStore::new();
        // No-op rebind before any root is configured: still
        // returns no-change.
        let changed = store
            .set_binding(
                HotkeyAction::RegionCapture,
                Some(HotkeyBindings::defaults().region_capture.clone().unwrap()),
            )
            .expect("set_binding");
        assert!(!changed);
    }

    #[test]
    fn set_paused_persists_across_reload() {
        let fs = IsolatedFilesystem::new("hotkey-pause").expect("fs");
        let store = HotkeyPreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let changed = store.set_paused(true).expect("set_paused");
        assert!(changed);

        let reloaded = HotkeyPreferencesStore::new();
        reloaded
            .set_root(fs.root().to_path_buf())
            .expect("set root");
        assert!(reloaded.current().paused);
        // Idempotent re-set returns no-change.
        let again = reloaded.set_paused(true).expect("set_paused");
        assert!(!again);
    }

    #[test]
    fn write_creates_backup_on_rotation() {
        let fs = IsolatedFilesystem::new("hotkey-rotation").expect("fs");
        let store = HotkeyPreferencesStore::new();
        store.set_root(fs.root().to_path_buf()).expect("set root");

        let mut first = HotkeyBindings::defaults();
        first.region_capture = Some("CommandOrControl+Alt+R".to_string());
        store.update(first).expect("first");
        let mut second = HotkeyBindings::defaults();
        second.shelf_toggle = Some("CommandOrControl+Alt+L".to_string());
        store.update(second).expect("second");

        let primary = fs::read_to_string(fs.join(HOTKEY_PRIMARY_FILENAME)).expect("primary");
        // The shelf_toggle binding is the most recent change and
        // is the only one mutated away from defaults; the primary
        // file must hold it.
        assert!(primary.contains("CommandOrControl+Alt+L"));
        let backup = fs::read_to_string(fs.join(HOTKEY_BACKUP_FILENAME)).expect("backup");
        // The backup holds the previous region binding.
        assert!(
            backup.contains("CommandOrControl+Alt+R"),
            "backup must hold the previous region binding"
        );
    }

    #[test]
    fn update_without_root_keeps_in_memory_state() {
        let store = HotkeyPreferencesStore::new();
        let mut next = HotkeyBindings::defaults();
        next.paused = true;
        store.update(next).expect("update without root");
        assert!(store.current().paused);
    }

    fn write_to_disk_for_seed(root: &Path, filename: &str, bindings: &HotkeyBindings) {
        let target = root.join(filename);
        let bytes = serde_json::to_vec_pretty(bindings).expect("serialise");
        std::fs::write(&target, &bytes).expect("write");
    }
}
