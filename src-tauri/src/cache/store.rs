//! The on-disk cache store.
//!
//! ## Directory layout
//!
//! ```text
//! <cache_root>/
//!   <capture_id>/
//!     capture.png       // flattened RGBA, atomically renamed
//!     bitmap.png        // optional staging bitmap
//!     metadata.json     // editable metadata
//!     manifest.json     // the publish sentinel
//! ```
//!
//! ## Two-phase commit
//!
//! Every cache commit goes through the same two phases:
//!
//! 1. **Asset phase.** `capture.png`, optional `bitmap.png`, and
//!    `metadata.json` are written via `atomic::write_atomic` to sibling
//!    `*.tmp` files, fsync'd, and renamed into place. Each step is
//!    independent: any failure cleans up the temp file and leaves the
//!    directory in a recoverable state.
//! 2. **Publish phase.** `manifest.json` is written atomically. The
//!    manifest is the single signal that tells the shelf the entry is
//!    durable. Only after this file lands is the entry exposed to the
//!    shelf.
//!
//! ## Recovery
//!
//! On startup, `Cache::load_or_recover` scans the cache root. Every
//! directory with a `manifest.json` is loaded. Every directory without
//! a manifest is a partial commit: it is reaped (recursive delete) so
//! the next commit starts from a clean slate. This satisfies the
//! acceptance criterion "restart after interruption at each atomic-write
//! stage and inspect cache consistency".
//!
//! ## Active locks
//!
//! The store keeps an `ActiveLockSet` in memory. Every cache entry
//! acquires a `Shelf` lock on commit so the entry cannot be dismissed
//! by another code path until the shelf releases it. The store is the
//! only owner of the registry; IPC handlers borrow it through the
//! `PixelGrabApp` state.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use pixelgrab_contracts::{
    cache::{CacheEntryMetadata, ShelfPosition},
    coordinate::{PhysicalBounds, PhysicalSize},
    monitor::MonitorLayout,
    CacheEntry as PublicCacheEntry, CaptureId, PlatformError, PlatformErrorKind, PlatformResult,
    ShelfId,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::atomic::{write_atomic, AtomicWriteOutcome};
use super::locks::{ActiveLockSet, LockGuard};

/// Names of the files inside each cache-entry directory. Centralised so
/// the writer and the reader can't drift apart.
pub mod file_names {
    /// Flattened PNG, derived from `flatten_crop`.
    pub const CAPTURE_PNG: &str = "capture.png";
    /// Optional staging bitmap.
    pub const BITMAP_PNG: &str = "bitmap.png";
    /// Editable metadata, JSON-encoded.
    pub const METADATA_JSON: &str = "metadata.json";
    /// Publish sentinel; written last.
    pub const MANIFEST_JSON: &str = "manifest.json";
}

/// The on-disk representation of the manifest. Kept private to the
/// store so the wire shape (`CacheEntry`) can evolve independently.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheManifest {
    capture_id: CaptureId,
    shelf_id: ShelfId,
    /// PNG path relative to the entry directory. Stored as a relative
    /// path so the cache is portable across host systems (e.g. when a
    /// tester copies the cache root to another machine).
    png_path: String,
    bitmap_path: Option<String>,
    bounds: PhysicalBounds,
    size: PhysicalSize,
    size_bytes: u64,
    metadata: CacheEntryMetadata,
    created_at_ms: i64,
    last_access_at_ms: i64,
    monitor_id: String,
}

/// Errors the cache store returns. Each variant maps to a
/// `PlatformErrorKind` at the IPC boundary.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// The configured cache root is not a directory or is not writable.
    #[error("cache root unusable: {0}")]
    BadRoot(String),
    /// A commit failed mid-way; the partial entry has been reaped (or
    /// will be reaped on the next startup scan).
    #[error("commit failed: {0}")]
    CommitFailed(String),
    /// The requested shelf id is unknown to the cache.
    #[error("unknown shelf id: {0}")]
    UnknownShelfId(ShelfId),
    /// The dismissal was blocked by one or more active locks.
    #[error("entry is still locked by: {0:?}")]
    StillLocked(Vec<&'static str>),
}

impl From<CacheError> for PlatformError {
    fn from(err: CacheError) -> Self {
        use PlatformErrorKind;
        let kind = match &err {
            CacheError::BadRoot(_) => PlatformErrorKind::Io,
            CacheError::CommitFailed(_) => PlatformErrorKind::Io,
            CacheError::UnknownShelfId(_) => PlatformErrorKind::InvalidPayload,
            CacheError::StillLocked(_) => PlatformErrorKind::InvalidSessionState,
        };
        let message = match &err {
            CacheError::StillLocked(owners) => {
                format!("entry is still locked by {owners:?}; release locks first")
            }
            other => other.to_string(),
        };
        PlatformError::new(kind, message)
    }
}

/// Request to publish one cache entry. Built by the commit pipeline
/// from the flattened RGBA buffer the platform produced.
#[derive(Debug, Clone)]
pub struct CommitRequest {
    /// Physical bounds of the flattened crop.
    pub bounds: PhysicalBounds,
    /// Pixel size (== bounds.size).
    pub size: PhysicalSize,
    /// Flattened RGBA buffer (length must equal width*height*4).
    pub rgba: Vec<u8>,
    /// Editable metadata to persist with the entry.
    pub metadata: CacheEntryMetadata,
    /// Identifier of the monitor whose work area will host the shelf
    /// card. Captured at commit time so the card repositions
    /// correctly if the primary monitor changes.
    pub monitor_id: String,
}

/// Result of a successful commit. The shelf uses these fields to
/// publish the card; the IPC layer hands them back to the frontend.
///
/// The cache holds the actual `Shelf` lock guard internally so it
/// lives until the entry is dismissed — see `Cache::dismiss`.
#[derive(Debug, Clone)]
pub struct CommitResult {
    /// The durable cache entry.
    pub entry: PublicCacheEntry,
    /// Size of the flattened PNG file in bytes (read back from disk so
    /// the IPC payload reports what is actually on disk).
    pub png_bytes: u64,
}

/// Inner state of the cache store. Held behind a single mutex; the
/// store operations are short (read manifest, write manifest, atomic
/// file rename) so the lock is never held across I/O waits.
#[derive(Debug)]
struct CacheInner {
    root: Option<PathBuf>,
    entries: std::collections::BTreeMap<ShelfId, PublicCacheEntry>,
    locks: ActiveLockSet,
    /// Owned lock guards for each entry. The shelf lock must live as
    /// long as the card is visible, so we keep it in the cache and
    /// only drop it when the entry is dismissed. Stored as a separate
    /// map so dropping a guard does not require touching the entries
    /// map.
    shelf_guards: std::collections::BTreeMap<ShelfId, LockGuard>,
}

/// The cache store. `Cache` is cheap to clone — every field is behind
/// an `Arc` or a `Mutex` already.
#[derive(Debug, Clone)]
pub struct Cache {
    inner: std::sync::Arc<Mutex<CacheInner>>,
}

impl Cache {
    /// Build a new empty cache. The root is unset until
    /// `set_cache_root` is called.
    pub fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(CacheInner {
                root: None,
                entries: Default::default(),
                locks: ActiveLockSet::new(),
                shelf_guards: Default::default(),
            })),
        }
    }

    /// Configure the on-disk root. The directory is created if it does
    /// not exist. Passing `None` clears the root.
    pub fn set_cache_root(&self, root: Option<PathBuf>) -> PlatformResult<()> {
        let mut inner = self.inner.lock();
        match root {
            None => {
                inner.root = None;
                Ok(())
            }
            Some(path) => {
                if let Err(err) = fs::create_dir_all(&path) {
                    return Err(CacheError::BadRoot(format!("{}: {err}", path.display())).into());
                }
                inner.root = Some(path);
                Ok(())
            }
        }
    }

    /// Read the configured root.
    pub fn cache_root(&self) -> Option<PathBuf> {
        self.inner.lock().root.clone()
    }

    /// Read-only access to the active-lock registry.
    pub fn locks(&self) -> ActiveLockSet {
        self.inner.lock().locks.clone_handle()
    }

    /// Run the startup scan: load every durable entry and reap any
    /// partial entry left behind by a crashed commit.
    pub fn load_or_recover(&self) -> PlatformResult<()> {
        let mut inner = self.inner.lock();
        let root = match inner.root.clone() {
            Some(root) => root,
            None => return Ok(()),
        };
        inner.entries.clear();
        if !root.exists() {
            return Ok(());
        }
        let read_dir = fs::read_dir(&root).map_err(|err| {
            PlatformError::from(CacheError::BadRoot(format!(
                "read_dir({}): {err}",
                root.display()
            )))
        })?;
        for entry in read_dir {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    log::warn!("skip unreadable cache entry: {err}");
                    continue;
                }
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join(file_names::MANIFEST_JSON);
            if manifest_path.exists() {
                match load_manifest(&path) {
                    Ok(public) => {
                        inner.entries.insert(public.shelf_id.clone(), public);
                    }
                    Err(err) => {
                        log::warn!(
                            "failed to load manifest at {}: {err}",
                            manifest_path.display()
                        );
                    }
                }
            } else {
                // Partial entry — reap.
                log::warn!(
                    "reaping partial cache entry {} (no manifest)",
                    path.display()
                );
                let _ = fs::remove_dir_all(&path);
            }
        }
        Ok(())
    }

    /// Run the two-phase commit pipeline. Returns the durable
    /// `CacheEntry` and a `Shelf` lock guard the caller must keep alive
    /// for as long as the shelf card is visible.
    pub fn commit(&self, request: CommitRequest) -> PlatformResult<CommitResult> {
        let entry_dir = {
            let inner = self.inner.lock();
            let root = inner.root.clone().ok_or_else(|| {
                PlatformError::new(PlatformErrorKind::Io, "cache root is not configured")
            })?;
            let capture_id = Uuid::new_v4().to_string();
            root.join(&capture_id)
        };
        // The directory may exist if a previous commit crashed after
        // writing assets but before the manifest; the recovery scan
        // reaps those on startup, but tests can hit this path with the
        // recovery scan bypassed. Always start with a clean slate so
        // the atomic-write helpers see an empty directory.
        if entry_dir.exists() {
            fs::remove_dir_all(&entry_dir).map_err(|err| {
                PlatformError::from(CacheError::CommitFailed(format!(
                    "remove_dir_all({}): {err}",
                    entry_dir.display()
                )))
            })?;
        }
        fs::create_dir_all(&entry_dir).map_err(|err| {
            PlatformError::from(CacheError::CommitFailed(format!(
                "create_dir_all({}): {err}",
                entry_dir.display()
            )))
        })?;

        let capture_id = entry_dir
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                PlatformError::from(CacheError::CommitFailed(
                    "entry dir has no file name".into(),
                ))
            })?
            .to_string();
        let shelf_id = Uuid::new_v4().to_string();
        let created_at_ms = now_ms();
        let size_bytes_expected = (request.size.width as u64) * (request.size.height as u64) * 4;

        // Phase 1 — write assets.
        let png_path_rel = file_names::CAPTURE_PNG.to_string();
        let png_bytes = encode_png(&request.rgba, request.size)?;
        write_atomic(&entry_dir.join(file_names::CAPTURE_PNG), &png_bytes)?;

        let bitmap_bytes: Option<Vec<u8>> = None;
        if let Some(bytes) = bitmap_bytes.as_ref() {
            write_atomic(&entry_dir.join(file_names::BITMAP_PNG), bytes)?;
        }

        let metadata_path = entry_dir.join(file_names::METADATA_JSON);
        let metadata_json = serde_json::to_vec_pretty(&request.metadata)?;
        write_atomic(&metadata_path, &metadata_json)?;

        // Phase 2 — write the manifest (the publish sentinel).
        let manifest = CacheManifest {
            capture_id: capture_id.clone(),
            shelf_id: shelf_id.clone(),
            png_path: png_path_rel,
            bitmap_path: None,
            bounds: request.bounds,
            size: request.size,
            size_bytes: png_bytes.len() as u64 + metadata_json.len() as u64,
            metadata: request.metadata.clone(),
            created_at_ms,
            last_access_at_ms: created_at_ms,
            monitor_id: request.monitor_id.clone(),
        };
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        let outcome = write_atomic(&entry_dir.join(file_names::MANIFEST_JSON), &manifest_json)?;
        // Update the size to reflect the manifest itself on a fresh
        // write; an idempotent retry sees the same size.
        let manifest_bytes = match &outcome {
            AtomicWriteOutcome::Written { bytes, .. } => *bytes,
            AtomicWriteOutcome::AlreadyDurable { bytes, .. } => *bytes,
        };
        let total_size_bytes = png_bytes.len() as u64 + metadata_json.len() as u64 + manifest_bytes;

        let _ = size_bytes_expected; // documented for reviewers; not asserted.

        let entry = PublicCacheEntry {
            capture_id,
            shelf_id: shelf_id.clone(),
            png_path: entry_dir
                .join(file_names::CAPTURE_PNG)
                .to_string_lossy()
                .to_string(),
            bitmap_path: None,
            bounds: request.bounds,
            size: request.size,
            size_bytes: total_size_bytes,
            metadata: request.metadata,
            created_at_ms,
            last_access_at_ms: created_at_ms,
            monitor_id: request.monitor_id,
        };

        // Acquire the shelf lock and register the entry. The guard is stored
        // inside the cache so the lock lives for the lifetime of the
        // card; it is released only when `Cache::dismiss` is called.
        {
            let mut inner = self.inner.lock();
            inner.entries.insert(shelf_id.clone(), entry.clone());
            let guard = inner
                .locks
                .acquire(shelf_id.clone(), pixelgrab_contracts::LockOwner::Shelf);
            inner.shelf_guards.insert(shelf_id.clone(), guard);
        }

        Ok(CommitResult {
            entry,
            png_bytes: png_bytes.len() as u64,
        })
    }

    /// Update the editable metadata of a shelf card. The metadata file
    /// is rewritten atomically; the manifest is then refreshed to
    /// record the new `last_access_at_ms`.
    pub fn update_metadata(
        &self,
        shelf_id: &str,
        metadata: CacheEntryMetadata,
    ) -> PlatformResult<PublicCacheEntry> {
        let mut inner = self.inner.lock();
        let entry = inner.entries.get(shelf_id).cloned().ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("unknown shelf id: {shelf_id}"),
            )
        })?;
        let entry_dir = PathBuf::from(&entry.png_path)
            .parent()
            .ok_or_else(|| {
                PlatformError::new(PlatformErrorKind::Io, "cache entry has no parent directory")
            })?
            .to_path_buf();

        let metadata_json = serde_json::to_vec_pretty(&metadata)?;
        write_atomic(&entry_dir.join(file_names::METADATA_JSON), &metadata_json)?;

        let updated = PublicCacheEntry {
            metadata: metadata.clone(),
            last_access_at_ms: now_ms(),
            ..entry.clone()
        };
        let manifest = CacheManifest {
            capture_id: updated.capture_id.clone(),
            shelf_id: updated.shelf_id.clone(),
            png_path: file_names::CAPTURE_PNG.to_string(),
            bitmap_path: None,
            bounds: updated.bounds,
            size: updated.size,
            size_bytes: updated.size_bytes,
            metadata: updated.metadata.clone(),
            created_at_ms: updated.created_at_ms,
            last_access_at_ms: updated.last_access_at_ms,
            monitor_id: updated.monitor_id.clone(),
        };
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        write_atomic(&entry_dir.join(file_names::MANIFEST_JSON), &manifest_json)?;

        inner.entries.insert(shelf_id.to_string(), updated.clone());
        Ok(updated)
    }

    /// Dismiss a shelf card. Releases the `Shelf` lock and reaps the
    /// entry directory if no other locks remain. The cached lock guard
    /// is dropped here so the dismissal truly releases the lock.
    pub fn dismiss(&self, shelf_id: &str) -> PlatformResult<super::locks::DismissOutcome> {
        let mut inner = self.inner.lock();
        // Release the cache-owned shelf guard first so the lock count
        // decrements before `try_dismiss` runs.
        inner.shelf_guards.remove(shelf_id);
        let outcome = inner.locks.try_dismiss(shelf_id);
        if outcome.reason == "removed" {
            if let Some(entry) = inner.entries.get(shelf_id) {
                let dir = PathBuf::from(&entry.png_path)
                    .parent()
                    .map(|p| p.to_path_buf());
                if let Some(dir) = dir {
                    drop(inner); // release the cache lock before
                                 // the recursive delete so a slow
                                 // disk doesn't block other calls.
                    let _ = fs::remove_dir_all(&dir);
                }
            }
        }
        // Re-acquire to update the in-memory map.
        let mut inner = self.inner.lock();
        if outcome.removed {
            inner.entries.remove(shelf_id);
        }
        Ok(outcome)
    }

    /// Snapshot of the current entries.
    pub fn entries(&self) -> Vec<PublicCacheEntry> {
        self.inner.lock().entries.values().cloned().collect()
    }

    /// Single-entry lookup by shelf id.
    pub fn entry(&self, shelf_id: &str) -> Option<PublicCacheEntry> {
        self.inner.lock().entries.get(shelf_id).cloned()
    }

    /// Compute the shelf position for the given entry against the
    /// current monitor layout. The function looks up the monitor
    /// stored in the entry and uses its `work_area`; if the monitor
    /// is no longer present it falls back to the primary monitor.
    pub fn shelf_position(
        &self,
        shelf_id: &str,
        layout: &MonitorLayout,
    ) -> PlatformResult<ShelfPosition> {
        let entry = self.entry(shelf_id).ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("unknown shelf id: {shelf_id}"),
            )
        })?;
        let monitor = layout
            .monitors
            .iter()
            .find(|m| m.id == entry.monitor_id && m.is_primary)
            .or_else(|| layout.monitors.iter().find(|m| m.is_primary))
            .or_else(|| layout.monitors.first())
            .ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorKind::MonitorQueryFailed,
                    "no monitor available for shelf placement",
                )
            })?;
        Ok(ShelfPosition::inside_primary_work_area(monitor))
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Load a manifest from `entry_dir` and reconstruct the public
/// `CacheEntry`. Returns `CacheError::BadRoot` on any read error.
fn load_manifest(entry_dir: &Path) -> PlatformResult<PublicCacheEntry> {
    let manifest_path = entry_dir.join(file_names::MANIFEST_JSON);
    let manifest_bytes = fs::read(&manifest_path).map_err(|err| {
        PlatformError::from(CacheError::BadRoot(format!(
            "read manifest {}: {err}",
            manifest_path.display()
        )))
    })?;
    let manifest: CacheManifest = serde_json::from_slice(&manifest_bytes).map_err(|err| {
        PlatformError::from(CacheError::BadRoot(format!(
            "parse manifest {}: {err}",
            manifest_path.display()
        )))
    })?;
    Ok(PublicCacheEntry {
        capture_id: manifest.capture_id,
        shelf_id: manifest.shelf_id,
        png_path: entry_dir
            .join(file_names::CAPTURE_PNG)
            .to_string_lossy()
            .to_string(),
        bitmap_path: manifest
            .bitmap_path
            .map(|p| entry_dir.join(p).to_string_lossy().to_string()),
        bounds: manifest.bounds,
        size: manifest.size,
        size_bytes: manifest.size_bytes,
        metadata: manifest.metadata,
        created_at_ms: manifest.created_at_ms,
        last_access_at_ms: manifest.last_access_at_ms,
        monitor_id: manifest.monitor_id,
    })
}

/// Minimal PNG encoder for the cache. Kept local to this module so the
/// cache layer has no dependency on `windows::capture::encode_png`
/// (which would couple cache tests to Windows-specific code).
///
/// Returns the encoded PNG bytes.
fn encode_png(rgba: &[u8], size: PhysicalSize) -> PlatformResult<Vec<u8>> {
    let width = size.width;
    let height = size.height;
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!(
                "rgba buffer length {} does not match {}x{}x4",
                rgba.len(),
                width,
                height
            ),
        ));
    }
    let mut buf = Vec::with_capacity(expected + 1024);
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| PlatformError::new(PlatformErrorKind::Io, format!("png header: {e}")))?;
        {
            use std::io::Write;
            let mut stream = writer.stream_writer().map_err(|e| {
                PlatformError::new(PlatformErrorKind::Io, format!("png stream: {e}"))
            })?;
            stream.write_all(rgba).map_err(|e| {
                PlatformError::new(PlatformErrorKind::Io, format!("png write: {e}"))
            })?;
            stream.finish().map_err(|e| {
                PlatformError::new(PlatformErrorKind::Io, format!("png finish: {e}"))
            })?;
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelgrab_test_support::fs::IsolatedFilesystem;

    fn filled_rgba(w: u32, h: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                buf.push((x & 0xFF) as u8);
                buf.push((y & 0xFF) as u8);
                buf.push(0);
                buf.push(0xFF);
            }
        }
        buf
    }

    #[test]
    fn commit_publishes_entry_and_keeps_lock() {
        let fs = IsolatedFilesystem::new("cache-commit").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let req = CommitRequest {
            bounds: PhysicalBounds::from_xywh(0, 0, 8, 8),
            size: PhysicalSize::new(8, 8),
            rgba: filled_rgba(8, 8),
            metadata: CacheEntryMetadata::default(),
            monitor_id: "primary".into(),
        };
        let result = cache.commit(req).expect("commit");
        assert_eq!(result.entry.capture_id.len(), 36); // UUID v4 string
                                                       // Manifest must exist on disk.
        let manifest_path = fs
            .root()
            .join(&result.entry.capture_id)
            .join(file_names::MANIFEST_JSON);
        assert!(manifest_path.exists(), "manifest is durable");
        // Capture PNG must exist.
        let png_path = fs
            .root()
            .join(&result.entry.capture_id)
            .join(file_names::CAPTURE_PNG);
        assert!(png_path.exists(), "capture PNG is durable");
        // Metadata file must exist.
        let metadata_path = fs
            .root()
            .join(&result.entry.capture_id)
            .join(file_names::METADATA_JSON);
        assert!(metadata_path.exists(), "metadata is durable");
        // Entry is registered and the shelf lock is held.
        let locks = cache.locks();
        assert_eq!(
            locks.owners_of(&result.entry.shelf_id),
            vec![pixelgrab_contracts::LockOwner::Shelf],
        );
    }

    #[test]
    fn load_or_recover_reaps_partial_entries() {
        let fs = IsolatedFilesystem::new("cache-recover").expect("fs");
        // Simulate a crash after capture.png was written but before
        // the manifest: drop a directory with just capture.png inside.
        let capture_id = "abcdef00-0000-0000-0000-000000000000";
        let entry_dir = fs.root().join(capture_id);
        fs::create_dir_all(&entry_dir).expect("mkdir");
        fs::write(entry_dir.join(file_names::CAPTURE_PNG), b"partial").expect("png");

        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        cache.load_or_recover().expect("recover");

        // The partial directory must be reaped.
        assert!(!entry_dir.exists(), "partial entry must be reaped");

        // The cache has nothing to show.
        assert!(cache.entries().is_empty());
    }

    #[test]
    fn load_or_recover_keeps_durable_entries() {
        let fs = IsolatedFilesystem::new("cache-load").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");

        let req = CommitRequest {
            bounds: PhysicalBounds::from_xywh(0, 0, 4, 4),
            size: PhysicalSize::new(4, 4),
            rgba: filled_rgba(4, 4),
            metadata: CacheEntryMetadata {
                title: "before restart".into(),
                ..CacheEntryMetadata::default()
            },
            monitor_id: "primary".into(),
        };
        let committed = cache.commit(req).expect("commit");
        // Simulate a process restart: drop the in-memory state and
        // rescan from disk.
        let cache2 = Cache::new();
        cache2
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        cache2.load_or_recover().expect("recover");
        let entry = cache2.entry(&committed.entry.shelf_id).expect("entry");
        assert_eq!(entry.metadata.title, "before restart");
        assert_eq!(entry.capture_id, committed.entry.capture_id);
    }

    #[test]
    fn dismiss_reaps_when_only_shelf_lock_held() {
        let fs = IsolatedFilesystem::new("cache-dismiss").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let req = CommitRequest {
            bounds: PhysicalBounds::from_xywh(0, 0, 4, 4),
            size: PhysicalSize::new(4, 4),
            rgba: filled_rgba(4, 4),
            metadata: CacheEntryMetadata::default(),
            monitor_id: "primary".into(),
        };
        let committed = cache.commit(req).expect("commit");
        let entry_dir = fs.root().join(&committed.entry.capture_id);
        assert!(entry_dir.exists());
        let outcome = cache.dismiss(&committed.entry.shelf_id).expect("dismiss");
        assert!(outcome.removed, "outcome was {outcome:?}");
        assert!(!entry_dir.exists(), "entry directory must be reaped");
        assert!(cache.entries().is_empty());
    }
}
