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
    CacheEntry as PublicCacheEntry, CaptureId, LockOwner, PlatformError, PlatformErrorKind,
    PlatformResult, RevisionMetadata, ShelfId, REVISION_SCHEMA_VERSION,
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
    /// Tracer-10: editor scene (annotations, badge counter, tool /
    /// style state). Sibling of `metadata.json` so the user-facing
    /// metadata schema and the editor-scene schema can evolve
    /// independently.
    pub const REVISION_JSON: &str = "revision.json";
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
}

impl From<CacheError> for PlatformError {
    fn from(err: CacheError) -> Self {
        use PlatformErrorKind;
        let kind = match &err {
            CacheError::BadRoot(_) => PlatformErrorKind::Io,
            CacheError::CommitFailed(_) => PlatformErrorKind::Io,
            CacheError::UnknownShelfId(_) => PlatformErrorKind::InvalidPayload,
        };
        PlatformError::new(kind, err.to_string())
    }
}

/// Request to publish one cache entry. Built by the commit pipeline
/// from the flattened RGBA buffer the platform produced. Named
/// `CacheCommitRequest` (not `CommitRequest`) to avoid colliding with
/// `pixelgrab_contracts::ipc::CommitRequest` at the IPC call site.
#[derive(Debug, Clone)]
pub struct CacheCommitRequest {
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

/// Stage at which an injected failure should fire. Used by
/// `failure-injection` tests that exercise each commit step in
/// isolation. The stages are listed in the order they run during
/// `Cache::commit`. The enum is compiled in non-test builds too so
/// the integration test crate can exercise the fault API; the
/// production commit path never sets `pending_failure`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommitStage {
    /// `create_dir_all` on the entry directory.
    CreateDir,
    /// Encoding the flattened RGBA buffer to PNG bytes.
    EncodePng,
    /// The atomic PNG write.
    WritePng,
    /// The atomic metadata write.
    WriteMetadata,
    /// The atomic manifest write (the publish sentinel).
    WriteManifest,
    /// The `file_size` stat that follows the asset phase.
    ReadOnDiskSize,
}

/// Fault injected at a specific commit stage. The `Cache` test API
/// arms faults with `Cache::arm_failure`; the fault fires once and
/// is then cleared.
#[derive(Debug)]
pub struct InjectedFailure {
    /// Stage at which the fault should fire.
    pub stage: CommitStage,
    /// Error to surface from the chosen stage.
    pub error: PlatformError,
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
    /// Tracer-10: owned `Editor` lock guards. The editor lock is
    /// acquired when a user opens a shelf entry for non-destructive
    /// editing and released on commit success or cancel. Storing it
    /// here keeps the lock alive for the lifetime of the reopen
    /// session so the sweeper and the manual `clear_cache` cannot
    /// evict the user's work-in-progress.
    editor_guards: std::collections::BTreeMap<ShelfId, LockGuard>,
    /// Issue #63: owned `Pin` lock guards, one per open pin reference.
    /// The pin registry's cache-backed lock provider pushes a guard on
    /// every `acquire(capture_id)` and pops one on every `release`, so
    /// N pins of the same capture keep N references alive. While any
    /// guard is held the sweeper and the manual `clear_cache` cannot
    /// evict the pinned entry's PNG.
    pin_guards: std::collections::BTreeMap<ShelfId, Vec<LockGuard>>,
    /// Optional one-shot fault for the next commit. `None` in
    /// production; tests set this to exercise a specific stage.
    pending_failure: Option<InjectedFailure>,
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
                editor_guards: Default::default(),
                pin_guards: Default::default(),
                pending_failure: None,
            })),
        }
    }

    /// Arm a one-shot failure for the next `commit` call. The
    /// failure fires when `commit` reaches `stage`; if the next
    /// commit does not reach that stage, the failure is left armed
    /// and will fire on the following commit instead. Tests should
    /// call this with a fresh cache per scenario to avoid bleed.
    ///
    /// The API is available in production builds so the integration
    /// test crate can drive it; the production commit path leaves
    /// `pending_failure` as `None`, so the runtime cost is a single
    /// `Option` discriminant check per commit stage.
    pub fn arm_failure(&self, stage: CommitStage, error: PlatformError) {
        let mut inner = self.inner.lock();
        inner.pending_failure = Some(InjectedFailure { stage, error });
    }

    /// Test helper: take the pending failure (if any) and clear it.
    fn take_failure(&self, stage: CommitStage) -> Option<PlatformError> {
        let mut inner = self.inner.lock();
        if let Some(pending) = inner.pending_failure.as_ref() {
            if pending.stage == stage {
                let err = inner.pending_failure.take().unwrap();
                return Some(err.error);
            }
        }
        None
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
    pub fn commit(&self, request: CacheCommitRequest) -> PlatformResult<CommitResult> {
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
        if let Some(err) = self.take_failure(CommitStage::CreateDir) {
            return Err(err);
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

        // Phase 1 — write assets. The whole asset phase is wrapped in
        // a closure so any failure reaps the partial directory before
        // propagating the error. The shelf only ever sees a commit
        // after the manifest has landed, so a partial directory left
        // behind would be a phantom-card regression.
        let asset_outcome: Result<Vec<u8>, PlatformError> =
            (|| -> Result<Vec<u8>, PlatformError> {
                if let Some(err) = self.take_failure(CommitStage::EncodePng) {
                    return Err(err);
                }
                let png_bytes = encode_png(&request.rgba, request.size)?;
                if let Some(err) = self.take_failure(CommitStage::WritePng) {
                    return Err(err);
                }
                write_atomic(&entry_dir.join(file_names::CAPTURE_PNG), &png_bytes)?;
                let metadata_json = serde_json::to_vec_pretty(&request.metadata)?;
                if let Some(err) = self.take_failure(CommitStage::WriteMetadata) {
                    return Err(err);
                }
                write_atomic(&entry_dir.join(file_names::METADATA_JSON), &metadata_json)?;
                // Tracer-10: write the initial empty revision sidecar
                // so a reopen session has a baseline to round-trip.
                // The wrapper `AssetWrite` stage keeps the existing
                // fault-injection contract intact — every prior
                // failure test still uses the same anchors.
                let initial_revision = RevisionMetadata::empty(
                    shelf_id.clone(),
                    capture_id.clone(),
                    request.bounds,
                    request.size,
                );
                let revision_json = serde_json::to_vec_pretty(&initial_revision)?;
                write_atomic(&entry_dir.join(file_names::REVISION_JSON), &revision_json)?;
                Ok(png_bytes)
            })();

        let png_bytes = match asset_outcome {
            Ok(bytes) => bytes,
            Err(err) => {
                // Phase 1 failed — reap the partial directory so no
                // orphaned files survive the IPC call.
                let _ = fs::remove_dir_all(&entry_dir);
                return Err(err);
            }
        };

        // Phase 2 — write the manifest (the publish sentinel). The
        // manifest does not store its own byte size; the cache
        // computes `size_bytes` from the on-disk file sizes when it
        // loads an entry, so the manifest and the in-memory
        // `PublicCacheEntry` cannot drift.
        let manifest = CacheManifest {
            capture_id: capture_id.clone(),
            shelf_id: shelf_id.clone(),
            png_path: file_names::CAPTURE_PNG.to_string(),
            bitmap_path: None,
            bounds: request.bounds,
            size: request.size,
            metadata: request.metadata.clone(),
            created_at_ms,
            last_access_at_ms: created_at_ms,
            monitor_id: request.monitor_id.clone(),
        };
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        if let Some(err) = self.take_failure(CommitStage::WriteManifest) {
            // Phase 2 failed — reap the partial directory.
            let _ = fs::remove_dir_all(&entry_dir);
            return Err(err);
        }
        let manifest_outcome =
            write_atomic(&entry_dir.join(file_names::MANIFEST_JSON), &manifest_json);
        let manifest_bytes = match manifest_outcome {
            Ok(AtomicWriteOutcome::Written { bytes, .. }) => bytes,
            Ok(AtomicWriteOutcome::AlreadyDurable { bytes, .. }) => bytes,
            Err(err) => {
                // Phase 2 failed — reap the partial directory.
                let _ = fs::remove_dir_all(&entry_dir);
                return Err(err);
            }
        };

        // Read the on-disk file sizes so `size_bytes` reflects the
        // bytes the cache will actually serve on restart, not an
        // estimate from in-memory buffer lengths.
        if let Some(err) = self.take_failure(CommitStage::ReadOnDiskSize) {
            let _ = fs::remove_dir_all(&entry_dir);
            return Err(err);
        }
        let png_size = file_size(&entry_dir.join(file_names::CAPTURE_PNG))?;
        let metadata_size = file_size(&entry_dir.join(file_names::METADATA_JSON))?;
        let revision_size = file_size(&entry_dir.join(file_names::REVISION_JSON))?;
        let total_size_bytes = png_size + metadata_size + revision_size + manifest_bytes;

        let entry = PublicCacheEntry {
            capture_id: capture_id.clone(),
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

        // Read the on-disk file sizes so the persisted manifest and
        // the in-memory entry agree on `size_bytes`.
        let png_size = file_size(&entry_dir.join(file_names::CAPTURE_PNG))?;
        let metadata_size = file_size(&entry_dir.join(file_names::METADATA_JSON))?;
        let revision_size = file_size(&entry_dir.join(file_names::REVISION_JSON))?;

        let updated_metadata_only = PublicCacheEntry {
            metadata: metadata.clone(),
            last_access_at_ms: now_ms(),
            ..entry.clone()
        };
        let manifest = CacheManifest {
            capture_id: updated_metadata_only.capture_id.clone(),
            shelf_id: updated_metadata_only.shelf_id.clone(),
            png_path: file_names::CAPTURE_PNG.to_string(),
            bitmap_path: None,
            bounds: updated_metadata_only.bounds,
            size: updated_metadata_only.size,
            metadata: updated_metadata_only.metadata.clone(),
            created_at_ms: updated_metadata_only.created_at_ms,
            last_access_at_ms: updated_metadata_only.last_access_at_ms,
            monitor_id: updated_metadata_only.monitor_id.clone(),
        };
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        let manifest_bytes =
            match write_atomic(&entry_dir.join(file_names::MANIFEST_JSON), &manifest_json)? {
                AtomicWriteOutcome::Written { bytes, .. } => bytes,
                AtomicWriteOutcome::AlreadyDurable { bytes, .. } => bytes,
            };
        let total_size_bytes = png_size + metadata_size + revision_size + manifest_bytes;
        let updated = PublicCacheEntry {
            size_bytes: total_size_bytes,
            ..updated_metadata_only.clone()
        };

        inner.entries.insert(shelf_id.to_string(), updated.clone());
        Ok(updated)
    }

    /// Dismiss a shelf card. Releases the `Shelf` lock and reaps the
    /// entry directory if no other locks remain. The cached lock guard
    /// is dropped here so the dismissal truly releases the lock.
    pub fn dismiss(&self, shelf_id: &str) -> PlatformResult<super::locks::DismissOutcome> {
        // Decide the directory to remove (if any) BEFORE taking the
        // lock, so the recursive delete runs lock-free. The cache's
        // mutex is not re-entrant, so we cannot hold it across the
        // `fs::remove_dir_all` call and re-acquire afterward — we
        // must drop it first.
        let dir_to_remove: Option<PathBuf> = {
            let inner = self.inner.lock();
            inner
                .entries
                .get(shelf_id)
                .map(|entry| PathBuf::from(&entry.png_path))
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        };
        {
            let mut inner = self.inner.lock();
            // Release the cache-owned shelf guard first so the lock
            // count decrements before `try_dismiss` runs.
            inner.shelf_guards.remove(shelf_id);
            // Tracer-10: any editor lock is owned by the cache so we
            // must drop the guard here too — otherwise an entry
            // dismissed via the shelf path would leak the editor
            // lock until the process restarts.
            inner.editor_guards.remove(shelf_id);
            let outcome = inner.locks.try_dismiss(shelf_id);
            if outcome.removed {
                inner.entries.remove(shelf_id);
            }
            // `inner` is dropped here so the recursive delete below
            // doesn't hold the cache lock.
            drop(inner);
            if outcome.removed {
                if let Some(dir) = &dir_to_remove {
                    let _ = fs::remove_dir_all(dir);
                }
            }
            Ok(outcome)
        }
    }
    /// Snapshot of the current entries.
    pub fn entries(&self) -> Vec<PublicCacheEntry> {
        self.inner.lock().entries.values().cloned().collect()
    }

    // -----------------------------------------------------------------------
    // Tracer-10: revision lifecycle. The helpers below are the storage
    // half of the reopen / commit / cancel flow. The IPC layer in
    // `src-tauri/src/ipc/commands.rs` is the only caller.
    // -----------------------------------------------------------------------

    /// Read the revision sidecar for the given shelf id. Returns
    /// `None` when the entry is unknown, the file is missing,
    /// the JSON is unparseable, or the schema version is
    /// unsupported — the caller is responsible for choosing
    /// between the full editor and the flat-PNG fallback.
    pub fn read_revision(&self, shelf_id: &str) -> Option<RevisionMetadata> {
        let entry = self.entry(shelf_id)?;
        let entry_dir = PathBuf::from(&entry.png_path).parent()?.to_path_buf();
        let path = entry_dir.join(file_names::REVISION_JSON);
        let bytes = fs::read(&path).ok()?;
        let parsed = serde_json::from_slice::<RevisionMetadata>(&bytes).ok()?;
        // Reject unsupported versions BEFORE sanitizing so the
        // IPC layer can fall back to the flat-PNG path. The
        // acceptance criterion "Unsupported or missing metadata
        // degrades safely to flattened-image editing" hinges on
        // this branch.
        if parsed.schema_version != REVISION_SCHEMA_VERSION {
            return None;
        }
        Some(parsed.sanitize())
    }

    /// Write the revision sidecar for the given shelf id. Refuses
    /// to write when the entry is unknown so a stale shelf id
    /// cannot create a stray sidecar.
    ///
    /// The revision contents are expected to change on every
    /// edit (badge counter, annotations, tool state), so this
    /// helper deletes the existing file before delegating to
    /// `write_atomic`. The delete + write pair is not atomic on
    /// its own — a crash window between the two operations
    /// leaves a missing file, which the loader treats as a flat
    /// fallback. That is the correct behavior: the in-progress
    /// scene is recoverable from the user's next edit, not from
    /// a stale persisted file.
    pub fn write_revision(
        &self,
        shelf_id: &str,
        revision: &RevisionMetadata,
    ) -> PlatformResult<RevisionMetadata> {
        let entry_dir = {
            let inner = self.inner.lock();
            let entry = inner.entries.get(shelf_id).cloned().ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorKind::InvalidPayload,
                    format!("unknown shelf id: {shelf_id}"),
                )
            })?;
            PathBuf::from(&entry.png_path)
                .parent()
                .ok_or_else(|| {
                    PlatformError::new(PlatformErrorKind::Io, "cache entry has no parent directory")
                })?
                .to_path_buf()
        };
        let sanitized = revision.sanitize();
        let json = serde_json::to_vec_pretty(&sanitized)?;
        let target = entry_dir.join(file_names::REVISION_JSON);
        // `write_atomic` refuses to overwrite a file with
        // different bytes; the revision payload changes on every
        // edit, so we delete the existing file first.
        let _ = fs::remove_file(&target);
        write_atomic(&target, &json)?;
        // Refresh `last_access_at_ms` so the sweeper sees the user's
        // recent attention.
        let mut inner = self.inner.lock();
        if let Some(entry) = inner.entries.get_mut(shelf_id) {
            entry.last_access_at_ms = now_ms();
        }
        Ok(sanitized)
    }

    /// Acquire the `Editor` lock on the given entry. The lock guard
    /// is stored in the cache so it lives for the lifetime of the
    /// reopen session. Returns the locked entry's current revision
    /// payload (or `None` when the sidecar is missing / unparseable)
    /// so the IPC layer can fold the two reads into one IPC round.
    ///
    /// Returns `Err` with `InvalidPayload` when the entry is
    /// unknown. The `Editor` lock is idempotent: re-acquiring it
    /// for an already-locked entry is a no-op (the active-lock
    /// registry treats the same `(shelf_id, owner)` pair as a
    /// single lock).
    pub fn acquire_editor_lock(
        &self,
        shelf_id: &str,
    ) -> PlatformResult<(PublicCacheEntry, Option<RevisionMetadata>)> {
        let mut inner = self.inner.lock();
        let entry = inner.entries.get(shelf_id).cloned().ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("unknown shelf id: {shelf_id}"),
            )
        })?;
        // Acquire the lock guard only if not already held. The
        // underlying registry is idempotent, but we store one guard
        // per shelf id — a second `insert` would replace the prior
        // guard and silently change the lock release timing.
        if !inner.editor_guards.contains_key(shelf_id) {
            let guard = inner.locks.acquire(shelf_id.to_string(), LockOwner::Editor);
            inner.editor_guards.insert(shelf_id.to_string(), guard);
        }
        // Read the revision sidecar off-disk (lock is held across
        // the read so the response is consistent with the lock
        // state). Tolerate a missing or unparseable file —
        // `read_revision` returns `None` in that case.
        let entry_dir = PathBuf::from(&entry.png_path)
            .parent()
            .ok_or_else(|| {
                PlatformError::new(PlatformErrorKind::Io, "cache entry has no parent directory")
            })?
            .to_path_buf();
        let path = entry_dir.join(file_names::REVISION_JSON);
        let revision = match fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RevisionMetadata>(&bytes).ok())
        {
            Some(r) if r.schema_version == REVISION_SCHEMA_VERSION => Some(r.sanitize()),
            _ => None,
        };
        Ok((entry, revision))
    }

    /// Release the `Editor` lock on the given entry. Idempotent:
    /// releasing a lock that is not held is a no-op, mirroring the
    /// `ActiveLockSet::release` semantics.
    pub fn release_editor_lock(&self, shelf_id: &str) {
        let mut inner = self.inner.lock();
        inner.editor_guards.remove(shelf_id);
        // The wrapper map drops the owned `LockGuard`, which fires
        // the underlying `registry.release` on Drop. This is the
        // parking_lot-safe pattern (the cache's mutex is
        // non-reentrant).
    }

    /// True when the given entry currently holds an `Editor` lock.
    /// Used by the IPC layer to reject a `commit_revision` /
    /// `cancel_revision` that lacks the lock.
    pub fn has_editor_lock(&self, shelf_id: &str) -> bool {
        self.inner.lock().editor_guards.contains_key(shelf_id)
    }

    // ------------------------------------------------------------------
    // Issue #63: pin + drag ownership on the shared lock registry.
    // ------------------------------------------------------------------

    /// Resolve a committed entry by its capture id. The pin registry's
    /// cache-backed lock provider keys its `PinLockProvider` contract
    /// on `capture_id`, so the mapping from pin source → shelf entry
    /// lives here (capture ids are UUIDv4 strings and are unique per
    /// committed entry).
    pub fn entry_by_capture(&self, capture_id: &str) -> Option<PublicCacheEntry> {
        let inner = self.inner.lock();
        inner
            .entries
            .values()
            .find(|entry| entry.capture_id == capture_id)
            .cloned()
    }

    /// Reap orphaned capture-frame assets written by the bounded
    /// transport (issue #63). Frames live under `<root>/frames/` and
    /// are outside the entry enumeration, so without this pass they
    /// would grow unboundedly past the cache's TTL.
    ///
    /// - `None`: reap every frame regardless of age (startup recovery
    ///   — no frozen frame can survive a process restart).
    /// - `Some(max_age_ms)`: reap only frames whose mtime is older
    ///   than the bound (periodic sweep; a live capture session
    ///   finishes in seconds, so anything older is debris).
    ///
    /// Returns the bytes reclaimed. Per-file failures are skipped so
    /// one locked file cannot strand the rest.
    pub fn reap_frame_assets(&self, max_age_ms: Option<i64>, now_ms: i64) -> u64 {
        let Some(root) = self.inner.lock().root.clone() else {
            return 0;
        };
        let frames_dir = root.join("frames");
        let Ok(entries) = fs::read_dir(&frames_dir) else {
            return 0;
        };
        let mut reclaimed = 0u64;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(max_age) = max_age_ms {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let Ok(modified) = metadata.modified() else {
                    continue;
                };
                let modified_ms = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if now_ms.saturating_sub(modified_ms) < max_age {
                    continue;
                }
            }
            if let Ok(size) = remove_file_with_size(&path) {
                reclaimed = reclaimed.saturating_add(size);
            }
        }
        // The directory itself is removed when empty so a fully reaped
        // frames dir leaves no residue.
        let _ = fs::remove_dir(&frames_dir);
        reclaimed
    }

    /// is stored inside the cache so it lives until the matching
    /// [`Cache::release_pin_guard`] pops it. Multiple pins of the same
    /// capture each hold their own reference; the entry stays protected
    /// from the sweeper and from dismissal until the last one drops.
    ///
    /// Returns `Err` with `InvalidPayload` when the entry is unknown.
    pub fn acquire_pin_guard(&self, shelf_id: &str) -> PlatformResult<()> {
        let mut inner = self.inner.lock();
        if !inner.entries.contains_key(shelf_id) {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("unknown shelf id: {shelf_id}"),
            ));
        }
        let guard = inner.locks.acquire(shelf_id.to_string(), LockOwner::Pin);
        inner
            .pin_guards
            .entry(shelf_id.to_string())
            .or_default()
            .push(guard);
        Ok(())
    }

    /// Release one `Pin` lock reference previously acquired with
    /// [`Cache::acquire_pin_guard`]. Returns `true` when a live
    /// reference was popped and `false` when the entry held none
    /// (mirrors the `PinLockProvider::release` refcount semantics).
    pub fn release_pin_guard(&self, shelf_id: &str) -> bool {
        let mut inner = self.inner.lock();
        match inner.pin_guards.get_mut(shelf_id) {
            Some(guards) => {
                guards.pop().is_some()
                // The popped `LockGuard` fires the underlying
                // `registry.release` on Drop — outside the cache mutex,
                // matching the editor-guard pattern above.
            }
            None => false,
        }
    }

    /// Number of live `Pin` lock references on the given entry. Used by
    /// tests and by the provider's `active_locks` accounting.
    pub fn pin_guard_count(&self, shelf_id: &str) -> usize {
        self.inner
            .lock()
            .pin_guards
            .get(shelf_id)
            .map(|guards| guards.len())
            .unwrap_or(0)
    }

    /// Acquire a short-lived `Drag` lock on the given entry for the
    /// duration of a synchronous OLE drag loop. The returned RAII guard
    /// releases the lock when dropped, so the IPC layer can wrap the
    /// blocking `start_drag` call without a manual rollback path.
    ///
    /// While the guard is alive the sweeper cannot evict the dragged
    /// PNG (AGENTS.md §14 invariant). Returns `Err` with
    /// `InvalidPayload` when the entry is unknown.
    pub fn acquire_drag_lock(&self, shelf_id: &str) -> PlatformResult<super::locks::LockGuard> {
        let inner = self.inner.lock();
        if !inner.entries.contains_key(shelf_id) {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("unknown shelf id: {shelf_id}"),
            ));
        }
        Ok(inner.locks.acquire(shelf_id.to_string(), LockOwner::Drag))
    }

    /// Read the absolute path of the entry's `revision.json` file.
    /// Used by the integration tests so the test can write a
    /// corrupt JSON to the sidecar and exercise the loader's
    /// fallback path.
    #[cfg(test)]
    pub fn revision_path_for(&self, shelf_id: &str) -> Option<PathBuf> {
        let entry = self.entry(shelf_id)?;
        let entry_dir = PathBuf::from(&entry.png_path).parent()?.to_path_buf();
        Some(entry_dir.join(file_names::REVISION_JSON))
    }

    /// Single-entry lookup by shelf id.
    pub fn entry(&self, shelf_id: &str) -> Option<PublicCacheEntry> {
        self.inner.lock().entries.get(shelf_id).cloned()
    }

    /// Live snapshot of the cache's usage. Used by the sweeper to
    /// decide whether pruning is needed and surfaced to the frontend
    /// via the `get_cache_stats` IPC.
    ///
    /// The `locked_count` counts entries that have at least one
    /// non-`Shelf` lock owner (editor / drag / pin). The sweeper
    /// skips these so the user's in-progress work is preserved.
    pub fn stats(&self) -> pixelgrab_contracts::CacheStats {
        let inner = self.inner.lock();
        let mut total_bytes: u64 = 0;
        let mut oldest_created_at_ms: Option<i64> = None;
        let mut newest_access_at_ms: Option<i64> = None;
        let mut locked_count: u32 = 0;
        for entry in inner.entries.values() {
            total_bytes = total_bytes.saturating_add(entry.size_bytes);
            oldest_created_at_ms = Some(
                oldest_created_at_ms
                    .map_or(entry.created_at_ms, |cur| cur.min(entry.created_at_ms)),
            );
            newest_access_at_ms =
                Some(newest_access_at_ms.map_or(entry.last_access_at_ms, |cur| {
                    cur.max(entry.last_access_at_ms)
                }));
            let owners = inner.locks.owners_of(&entry.shelf_id);
            if owners.iter().any(|o| *o != LockOwner::Shelf) {
                locked_count = locked_count.saturating_add(1);
            }
        }
        pixelgrab_contracts::CacheStats {
            total_bytes,
            entry_count: inner.entries.len() as u32,
            locked_count,
            oldest_created_at_ms,
            newest_access_at_ms,
        }
    }

    /// Look up the byte size of an entry from disk. The in-memory
    /// `size_bytes` is the source of truth for the stats and the
    /// sweeper, but the recovery sweep uses this to count the bytes
    /// it is about to reclaim so the `bytes_reclaimed` field is
    /// accurate even when the in-memory map is out of sync.
    pub fn entry_size_bytes(&self, shelf_id: &str) -> Option<u64> {
        self.inner
            .lock()
            .entries
            .get(shelf_id)
            .map(|e| e.size_bytes)
    }

    /// Read the on-disk size of an entry's PNG file. The sweeper
    /// uses this so `bytes_reclaimed` reflects the actual disk
    /// outcome at the moment of eviction, not the cached `size_bytes`
    /// which can drift from disk if a write was interrupted.
    pub fn entry_on_disk_size(&self, shelf_id: &str) -> Option<u64> {
        let inner = self.inner.lock();
        let entry = inner.entries.get(shelf_id)?;
        let path = std::path::PathBuf::from(&entry.png_path);
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            return None;
        }
        Some(size)
    }

    /// True when the entry is protected from the periodic sweeper
    /// and the manual clear. An entry is protected when it has any
    /// non-`Shelf` lock owner (editor / drag / pin). The default
    /// `Shelf` lock is the marker every commit acquires; it does
    /// NOT protect — otherwise no entry would ever be evictable.
    /// The spec's "active shelf" wording maps to the shelf queue
    /// engine's visible-set (the renderer pins a card via the
    /// `Shelf` lock for the duration of its visibility); the
    /// sweeper's intent is the underlying cache survival, which
    /// is governed by the explicit non-default owners.
    pub fn is_protected_from_sweeper(&self, shelf_id: &str) -> bool {
        let owners = self.inner.lock().locks.owners_of(shelf_id);
        owners.iter().any(|o| *o != LockOwner::Shelf)
    }

    /// Mark the provided entries as accessed at `now_ms`. Called by
    /// the shelf queue engine when a card is hovered or shown so the
    /// sweep's LRU order tracks the user's recent attention rather
    /// than the wall-clock time the entry was first committed.
    pub fn touch_entries(&self, shelf_ids: &[String], now_ms: i64) {
        let mut inner = self.inner.lock();
        for shelf_id in shelf_ids {
            if let Some(entry) = inner.entries.get_mut(shelf_id) {
                entry.last_access_at_ms = now_ms;
            }
        }
    }

    /// Sweep debris left behind by a crash. Variants removed:
    ///
    /// - Stale `*.tmp` files inside the cache root (atomic-write
    ///   leftovers from a commit that crashed mid-write).
    /// - Directories without a manifest (incomplete unindexed groups
    ///   from a crashed commit; the spec lists these alongside the
    ///   zero-byte assets and dangling temps).
    /// - Zero-byte `capture.png` or `metadata.json` files inside
    ///   entry directories.
    /// - Empty entry directories (no files at all) — including
    ///   manifest-present-but-no-assets corruption.
    ///
    /// Each category is reported separately so the caller can log a
    /// useful summary without leaking the cache root. The sweep
    /// continues past per-file failures so a single permission error
    /// on one file cannot strand the others.
    pub fn recover_debris(&self) -> PlatformResult<pixelgrab_contracts::SweepOutcome> {
        let root = self.inner.lock().root.clone();
        let Some(root) = root else {
            return Ok(pixelgrab_contracts::SweepOutcome::default());
        };
        let mut outcome = pixelgrab_contracts::SweepOutcome::default();
        if !root.exists() {
            return Ok(outcome);
        }
        let read_dir = match fs::read_dir(&root) {
            Ok(d) => d,
            Err(_err) => {
                // Privacy: never interpolate the path into the error
                // string (AGENTS.md §9). The cache root is allowed in
                // stable categorical kinds only.
                return Err(PlatformError::from(CacheError::BadRoot(
                    "read_dir failed".to_string(),
                )));
            }
        };
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_err) => {
                    outcome.partial_failures = outcome.partial_failures.saturating_add(1);
                    continue;
                }
            };
            let path = entry.path();
            if path.is_file() {
                // Stale `.tmp` file at the root (atomic-write leftover).
                if path.extension().and_then(|s| s.to_str()) == Some("tmp") {
                    match remove_file_with_size(&path) {
                        Ok(bytes) => {
                            outcome.tmp_files_removed = outcome.tmp_files_removed.saturating_add(1);
                            outcome.bytes_reclaimed = outcome.bytes_reclaimed.saturating_add(bytes);
                        }
                        Err(_err) => {
                            outcome.partial_failures = outcome.partial_failures.saturating_add(1);
                        }
                    }
                }
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join(file_names::MANIFEST_JSON);
            if manifest_path.exists() {
                // Durable entry — scan for zero-byte assets inside.
                outcome.zero_byte_assets_removed = outcome
                    .zero_byte_assets_removed
                    .saturating_add(reap_zero_byte_assets(&path));
                // Empty entry directory (manifest present but no
                // assets) is a corruption we can self-heal.
                outcome.unindexed_dirs_removed = outcome
                    .unindexed_dirs_removed
                    .saturating_add(reap_empty_assets(&path));
                continue;
            }
            // Manifest-less directory: per the spec, identify and
            // remove incomplete unindexed groups. Compute the bytes
            // before the recursive delete so the outcome reflects the
            // actual disk outcome even after the remove.
            //
            // Issue #63: the bounded transport's `frames/` directory is
            // not an entry group — it has no manifest by design and is
            // reaped on its own age policy (`reap_frame_assets`).
            // Skipping it here stops the debris pass from deleting
            // live capture frames.
            if path.file_name().and_then(|s| s.to_str()) == Some("frames") {
                continue;
            }
            match remove_dir_with_size(&path) {
                Ok(bytes) => {
                    outcome.unindexed_dirs_removed =
                        outcome.unindexed_dirs_removed.saturating_add(1);
                    outcome.bytes_reclaimed = outcome.bytes_reclaimed.saturating_add(bytes);
                }
                Err(_err) => {
                    outcome.partial_failures = outcome.partial_failures.saturating_add(1);
                }
            }
        }
        Ok(outcome)
    }

    /// Manually clear every *unlocked* entry. Used by the
    /// `clear_cache` IPC. Locked entries (editor / drag / pin
    /// owners) are not touched — the sweep outcome records the
    /// survivors so the UI can explain "X entries kept because
    /// they are in use". The default `Shelf` lock is the marker
    /// every commit acquires and does NOT protect — manual clear
    /// is meant to evict everything the user is not actively
    /// editing.
    ///
    /// `bytes_reclaimed` is computed from the on-disk PNG size at
    /// the moment of dismissal, not the cached `size_bytes`, so
    /// the UI can show accurate "reclaimed N bytes" feedback.
    pub fn clear_unlocked_entries(&self) -> pixelgrab_contracts::SweepOutcome {
        let mut outcome = pixelgrab_contracts::SweepOutcome::default();
        let shelf_ids: Vec<String> = self.inner.lock().entries.keys().cloned().collect();
        for shelf_id in shelf_ids {
            if self.is_protected_from_sweeper(&shelf_id) {
                // Locked by a non-default owner — keep it.
                continue;
            }
            let on_disk = self.entry_on_disk_size(&shelf_id);
            let outcome_one = match self.dismiss(&shelf_id) {
                Ok(o) => o,
                Err(_err) => {
                    outcome.partial_failures = outcome.partial_failures.saturating_add(1);
                    continue;
                }
            };
            if outcome_one.removed {
                outcome.quota_evicted = outcome.quota_evicted.saturating_add(1);
                if let Some(bytes) = on_disk {
                    outcome.bytes_reclaimed = outcome.bytes_reclaimed.saturating_add(bytes);
                }
            }
        }
        outcome
    }

    /// Resolve the primary monitor id for the given layout. Picks the
    /// first monitor that reports `is_primary`; falls back to the
    /// first monitor in the layout if no primary is reported.
    /// Returns `MonitorQueryFailed` if the layout is empty. Used by
    /// the commit pipeline (which needs a primary id) and by
    /// `shelf_position` (which prefers the entry's monitor if it is
    /// also the primary).
    pub fn primary_monitor_id(layout: &MonitorLayout) -> PlatformResult<String> {
        let id = layout
            .monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| layout.monitors.first())
            .map(|m| m.id.clone())
            .ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorKind::MonitorQueryFailed,
                    "no monitor available for shelf placement",
                )
            })?;
        Ok(id)
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
        // Prefer the entry's monitor when it is still present in the
        // layout. Fall back to the primary monitor via the shared
        // helper so primary-monitor selection has a single owner.
        let primary_id = Self::primary_monitor_id(layout)?;
        let monitor = layout
            .monitors
            .iter()
            .find(|m| m.id == entry.monitor_id)
            .or_else(|| layout.monitors.iter().find(|m| m.id == primary_id))
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

/// Read the byte size of a file at `path`. Returns the size or
/// converts the `io::Error` into a `PlatformError` so the caller can
/// surface the failure through the IPC layer.
fn file_size(path: &Path) -> PlatformResult<u64> {
    fs::metadata(path).map(|m| m.len()).map_err(|err| {
        PlatformError::from(CacheError::CommitFailed(format!(
            "stat({}): {err}",
            path.display()
        )))
    })
}

/// Load a manifest from `entry_dir` and reconstruct the public
/// `CacheEntry`. Returns `CacheError::BadRoot` on any read error.
/// `size_bytes` is recomputed from the on-disk file sizes so the
/// persisted manifest and the in-memory entry cannot drift.
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
    let png_size = file_size(&entry_dir.join(file_names::CAPTURE_PNG)).unwrap_or(0);
    let metadata_size = file_size(&entry_dir.join(file_names::METADATA_JSON)).unwrap_or(0);
    // `revision.json` is tracer-10; tolerate a missing file so an
    // entry pre-dating the sidecar (e.g. committed before the
    // branch landed) still loads cleanly.
    let revision_size = file_size(&entry_dir.join(file_names::REVISION_JSON)).unwrap_or(0);
    let total_size_bytes = png_size + metadata_size + revision_size + manifest_bytes.len() as u64;
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
        size_bytes: total_size_bytes,
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

/// Remove zero-byte `capture.png` or `metadata.json` files inside an
/// entry directory. Stale `*.tmp` siblings are also reaped. Returns
/// the number of bytes reclaimed. Failures are silently swallowed so
/// a single permission error does not strand the rest of the sweep.
fn reap_zero_byte_assets(entry_dir: &Path) -> u32 {
    let mut removed = 0u32;
    for name in [file_names::CAPTURE_PNG, file_names::METADATA_JSON] {
        let path = entry_dir.join(name);
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() == 0 {
            let _ = fs::remove_file(&path);
            removed = removed.saturating_add(1);
        }
    }
    // Stale `*.tmp` files inside an entry dir (atomic-write leftover).
    if let Ok(read_dir) = fs::read_dir(entry_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("tmp") {
                let _ = fs::remove_file(&path);
                removed = removed.saturating_add(1);
            }
        }
    }
    removed
}

/// Remove an entry directory that has a manifest but no asset files
/// (the cache can never repaint anything from such a directory). Used
/// as a self-heal path for the recovery sweep. Returns 1 when the
/// directory was removed, 0 otherwise.
fn reap_empty_assets(entry_dir: &Path) -> u32 {
    let has_assets = entry_dir.join(file_names::CAPTURE_PNG).exists()
        || entry_dir.join(file_names::METADATA_JSON).exists();
    if has_assets {
        return 0;
    }
    if fs::remove_dir_all(entry_dir).is_ok() {
        1
    } else {
        0
    }
}

/// Remove a file and return its size in bytes. Returns `Err` when
/// the file cannot be stat'd or removed. The size is what the
/// `SweepOutcome::bytes_reclaimed` field reports — the spec requires
/// reclaimed bytes to reflect the actual disk outcome.
fn remove_file_with_size(path: &Path) -> std::io::Result<u64> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    fs::remove_file(path)?;
    Ok(size)
}

/// Recursively remove a directory and return the cumulative on-disk
/// size of every file removed. Used by the recovery sweep when
/// reaping manifest-less entry directories and by the eviction
/// paths so `bytes_reclaimed` reflects what was actually deleted.
fn remove_dir_with_size(path: &Path) -> std::io::Result<u64> {
    let mut total: u64 = 0;
    if path.is_file() {
        total = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        fs::remove_file(path)?;
        return Ok(total);
    }
    if !path.is_dir() {
        return Ok(0);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            total = total.saturating_add(remove_dir_with_size(&child)?);
        } else {
            total = total.saturating_add(remove_file_with_size(&child).unwrap_or(0));
        }
    }
    fs::remove_dir(path)?;
    Ok(total)
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
    fn bad_root_display_carries_path() {
        // Regression guard for issue #52: the `set_cache_root` warn
        // log relies on `CacheError::BadRoot`'s `Display` carrying the
        // cache root path, so the setup hook can log `{err}` without
        // re-injecting the path. If this test ever fails, the
        // `src-tauri/src/lib.rs` startup log must be updated to keep
        // the path on the wire.
        let fs = IsolatedFilesystem::new("cache-bad-root").expect("fs");
        let file_path = fs.root().join("not-a-directory.txt");
        std::fs::write(&file_path, b"blocks the mkdir").expect("seed");
        let cache = Cache::new();
        let err = cache
            .set_cache_root(Some(file_path.clone()))
            .expect_err("set_cache_root should fail when the path is a file");
        let message = err.to_string();
        assert!(
            message.contains(file_path.display().to_string().as_str()),
            "BadRoot Display must carry the cache root path; got: {message}",
        );
        assert!(
            message.contains("cache root"),
            "BadRoot Display must keep the `cache root` prefix so the log remains self-describing; got: {message}",
        );
    }

    #[test]
    fn commit_publishes_entry_and_keeps_lock() {
        let fs = IsolatedFilesystem::new("cache-commit").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let req = CacheCommitRequest {
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

        let req = CacheCommitRequest {
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

    fn commit_request(size: u32) -> CacheCommitRequest {
        CacheCommitRequest {
            bounds: PhysicalBounds::from_xywh(0, 0, size, size),
            size: PhysicalSize::new(size, size),
            rgba: filled_rgba(size, size),
            metadata: CacheEntryMetadata::default(),
            monitor_id: "primary".into(),
        }
    }

    #[test]
    fn pin_guards_refcount_and_block_dismiss() {
        let fs = IsolatedFilesystem::new("cache-pin-lock").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let committed = cache.commit(commit_request(4)).expect("commit");
        let shelf_id = committed.entry.shelf_id.clone();

        // Two pins on the same capture each hold one reference.
        cache.acquire_pin_guard(&shelf_id).expect("first pin guard");
        cache
            .acquire_pin_guard(&shelf_id)
            .expect("second pin guard");
        assert_eq!(cache.pin_guard_count(&shelf_id), 2);

        // A pinned entry is protected from the sweeper and cannot be
        // dismissed while any pin guard is alive.
        assert!(cache.is_protected_from_sweeper(&shelf_id));
        let outcome = cache.dismiss(&shelf_id).expect("dismiss call");
        assert!(!outcome.removed, "pinned entry must survive dismissal");
        assert!(
            fs.root().join(&committed.entry.capture_id).exists(),
            "pinned entry assets must stay on disk"
        );

        // Release both refs; only the last release unlocks the entry.
        assert!(cache.release_pin_guard(&shelf_id));
        assert_eq!(cache.pin_guard_count(&shelf_id), 1);
        assert!(cache.is_protected_from_sweeper(&shelf_id));
        assert!(cache.release_pin_guard(&shelf_id));
        assert_eq!(cache.pin_guard_count(&shelf_id), 0);
        assert!(!cache.is_protected_from_sweeper(&shelf_id));

        // Releasing without a live ref is a no-op returning false.
        assert!(!cache.release_pin_guard(&shelf_id));

        let outcome = cache.dismiss(&shelf_id).expect("dismiss call");
        assert!(outcome.removed, "unpinned entry dismisses cleanly");
    }

    #[test]
    fn pin_guard_unknown_shelf_id_is_rejected() {
        let cache = Cache::new();
        let err = cache.acquire_pin_guard("missing").unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidPayload);
    }

    #[test]
    fn drag_guard_blocks_dismiss_until_dropped() {
        let fs = IsolatedFilesystem::new("cache-drag-lock").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let committed = cache.commit(commit_request(4)).expect("commit");
        let shelf_id = committed.entry.shelf_id.clone();

        let guard = cache.acquire_drag_lock(&shelf_id).expect("drag lock");
        assert!(cache.is_protected_from_sweeper(&shelf_id));
        let outcome = cache.dismiss(&shelf_id).expect("dismiss call");
        assert!(!outcome.removed, "entry under active drag survives");
        assert!(
            fs.root().join(&committed.entry.capture_id).exists(),
            "dragged PNG stays on disk for the OLE loop"
        );

        drop(guard);
        assert!(!cache.is_protected_from_sweeper(&shelf_id));
        let outcome = cache.dismiss(&shelf_id).expect("dismiss call");
        assert!(outcome.removed, "entry dismisses after the drag ends");
    }

    #[test]
    fn drag_lock_unknown_shelf_id_is_rejected() {
        let cache = Cache::new();
        let err = cache.acquire_drag_lock("missing").unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidPayload);
    }

    #[test]
    fn entry_by_capture_resolves_committed_entry() {
        let fs = IsolatedFilesystem::new("cache-by-capture").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let committed = cache.commit(commit_request(4)).expect("commit");

        let resolved = cache
            .entry_by_capture(&committed.entry.capture_id)
            .expect("entry resolves by capture id");
        assert_eq!(resolved.shelf_id, committed.entry.shelf_id);
        assert!(cache.entry_by_capture("no-such-capture").is_none());
    }

    #[test]
    fn dismiss_reaps_when_only_shelf_lock_held() {
        let fs = IsolatedFilesystem::new("cache-dismiss").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let req = CacheCommitRequest {
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
