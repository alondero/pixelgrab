//! Cache atomic-commit integration test. Drives the cache store
//! end-to-end via the synthetic platform and asserts:
//!
//! - Successful commits leave a manifest on disk and the cache holds a
//!   shelf lock.
//! - Failed commits (each stage injected) leave no manifest on disk
//!   and the cache has no entry.
//! - Restart after a partial commit reaps the partial directory.
//! - The cached PNG decodes to the same pixels as the input RGBA.
//!
//! The cache store exposes a one-shot fault-injection API
//! (`Cache::arm_failure`) so each commit stage can be exercised in
//! isolation. The helper is available in production builds because
//! the integration test crate compiles the cache as a regular
//! dependency; the cost is a single `Option` discriminant per
//! commit stage, and the field is never set outside tests.

use std::fs;
use std::sync::Arc;

use pixelgrab_contracts::{
    cache::CacheEntryMetadata,
    coordinate::{PhysicalBounds, PhysicalSize},
};
use pixelgrab_lib::cache::{Cache, CacheCommitRequest};

fn rgba(w: u32, h: u32) -> Vec<u8> {
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

fn test_request(w: u32, h: u32) -> CacheCommitRequest {
    CacheCommitRequest {
        bounds: PhysicalBounds::from_xywh(0, 0, w, h),
        size: PhysicalSize::new(w, h),
        rgba: rgba(w, h),
        metadata: CacheEntryMetadata::default(),
        monitor_id: "primary".into(),
    }
}

#[test]
fn commit_publishes_manifest_and_lock() {
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-test-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let result = cache.commit(test_request(4, 4)).expect("commit");
    let entry_dir = tmp.join(&result.entry.capture_id);
    assert!(
        entry_dir.join("manifest.json").exists(),
        "manifest must be durable",
    );
    assert!(
        entry_dir.join("capture.png").exists(),
        "capture.png must be durable",
    );
    assert!(
        entry_dir.join("metadata.json").exists(),
        "metadata.json must be durable",
    );
    let locks = cache.locks();
    let owners = locks.owners_of(&result.entry.shelf_id);
    assert_eq!(owners, vec![pixelgrab_contracts::LockOwner::Shelf]);
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn restart_reaps_partial_entry() {
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-recover-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    fs::create_dir_all(&tmp).expect("mkdir");
    // Simulate a crash after capture.png was written but before the
    // manifest landed: drop a directory with assets but no manifest.
    let capture_id = "01234567-89ab-cdef-0123-456789abcdef";
    let entry_dir = tmp.join(capture_id);
    fs::create_dir_all(&entry_dir).expect("mkdir");
    fs::write(entry_dir.join("capture.png"), b"partial-bytes").expect("write png");
    fs::write(entry_dir.join("metadata.json"), b"{}").expect("write metadata");

    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    cache.load_or_recover().expect("recover");
    assert!(!entry_dir.exists(), "partial entry must be reaped");
    assert!(cache.entries().is_empty());
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn restart_keeps_durable_entries() {
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-load-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let committed = cache.commit(test_request(4, 4)).expect("commit");
    let shelf_id = committed.entry.shelf_id.clone();

    // Simulate a process restart: new Cache, rescan.
    let cache2 = Cache::new();
    cache2.set_cache_root(Some(tmp.clone())).expect("set root");
    cache2.load_or_recover().expect("recover");
    let entry = cache2.entry(&shelf_id).expect("entry restored");
    assert_eq!(entry.capture_id, committed.entry.capture_id);
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn dismiss_with_only_shelf_lock_reaps_entry() {
    let cache = Arc::new(Cache::new());
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-dismiss-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let committed = cache.commit(test_request(4, 4)).expect("commit");
    let entry_dir = tmp.join(&committed.entry.capture_id);
    assert!(entry_dir.exists());
    let outcome = cache.dismiss(&committed.entry.shelf_id).expect("dismiss");
    assert!(outcome.removed);
    assert_eq!(outcome.reason, "removed");
    assert!(!entry_dir.exists(), "entry directory must be reaped");
    assert!(cache.entries().is_empty());
    fs::remove_dir_all(&tmp).ok();
}

// Tracer 07 acceptance criterion 5: "A visible card protects its
// backing assets from deletion." The integration version of this
// test was previously dropped on Windows flakiness; the rewritten
// version (after the cache rewrite that removed the cross-process
// wait-for-snapshot timing) runs reliably.
#[test]
fn shelf_owner_lock_protects_entry_on_disk() {
    let cache = Arc::new(Cache::new());
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-shelf-protects-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let committed = cache.commit(test_request(4, 4)).expect("commit");
    let entry_dir = tmp.join(&committed.entry.capture_id);
    assert!(
        entry_dir.exists(),
        "entry directory must be on disk after commit"
    );

    // The cache always holds a Shelf lock for the lifetime of the
    // card. `dismiss` releases that lock and reaps the entry only
    // when no other owner is present. A direct call to
    // `try_cleanup` must therefore be blocked.
    assert_eq!(
        cache.locks().try_cleanup(&committed.entry.shelf_id),
        pixelgrab_lib::cache::CleanupOutcome::StillLocked,
    );
    assert!(
        entry_dir.exists(),
        "entry directory must remain while the shelf lock is held",
    );

    // After the shelf lock is released by `dismiss`, cleanup is
    // empty and the entry is reaped from disk.
    let outcome = cache.dismiss(&committed.entry.shelf_id).expect("dismiss");
    assert!(outcome.removed, "shelf lock alone must allow dismissal");
    assert!(
        !entry_dir.exists(),
        "entry directory must be reaped after dismissal"
    );
    fs::remove_dir_all(&tmp).ok();
}

// Tracer 07 spec validation: "Commit captures while injecting
// directory creation, PNG write, metadata write, rename, and
// clipboard failures." The cache store exposes a one-shot fault
// injection API (`Cache::arm_failure`) so each stage can be exercised
// in isolation.
#[test]
fn failure_injection_at_create_dir_reaps_partial_and_surfaces_error() {
    use pixelgrab_contracts::PlatformErrorKind;
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-fault-create-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let io_err = pixelgrab_contracts::PlatformError::new(
        PlatformErrorKind::Io,
        "injected create_dir_all failure",
    );
    cache.arm_failure(pixelgrab_lib::cache::CommitStage::CreateDir, io_err);
    let result = cache.commit(test_request(4, 4));
    assert!(result.is_err(), "CreateDir failure must abort commit");
    let entry_dir = tmp.join("ignored");
    // No partial directory was created (the commit aborted before mkdir).
    let is_empty = fs::read_dir(&tmp)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    assert!(
        is_empty,
        "no entry directory should exist after CreateDir failure"
    );
    let _ = entry_dir;
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn failure_injection_at_write_png_reaps_partial() {
    use pixelgrab_contracts::PlatformErrorKind;
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-fault-png-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let io_err = pixelgrab_contracts::PlatformError::new(
        PlatformErrorKind::Io,
        "injected PNG write failure",
    );
    cache.arm_failure(pixelgrab_lib::cache::CommitStage::WritePng, io_err);
    let result = cache.commit(test_request(4, 4));
    assert!(result.is_err(), "WritePng failure must abort commit");
    // Partial directory must be reaped; no entry visible to the shelf.
    let entries: Vec<_> = fs::read_dir(&tmp)
        .map(|d| d.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    assert!(
        entries.is_empty(),
        "partial directory should be reaped on WritePng failure, found {:?}",
        entries.iter().map(|e| e.path()).collect::<Vec<_>>()
    );
    assert!(
        cache.entries().is_empty(),
        "no in-memory entry on WritePng failure"
    );
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn failure_injection_at_write_metadata_reaps_partial() {
    use pixelgrab_contracts::PlatformErrorKind;
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-fault-meta-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let io_err = pixelgrab_contracts::PlatformError::new(
        PlatformErrorKind::Io,
        "injected metadata write failure",
    );
    cache.arm_failure(pixelgrab_lib::cache::CommitStage::WriteMetadata, io_err);
    let result = cache.commit(test_request(4, 4));
    assert!(result.is_err(), "WriteMetadata failure must abort commit");
    let entries: Vec<_> = fs::read_dir(&tmp)
        .map(|d| d.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    assert!(
        entries.is_empty(),
        "partial directory should be reaped on WriteMetadata failure, found {:?}",
        entries.iter().map(|e| e.path()).collect::<Vec<_>>()
    );
    fs::remove_dir_all(&tmp).ok();
}

// Tracer 07 spec validation: "Compare card thumbnail, clipboard
// data, and cached PNG to the same expected flattened image." This
// integration test asserts that the PNG the cache stores on disk
// decodes back to the same RGBA the commit pipeline received (the
// `flatten_crop` output is the single source of truth for both the
// PNG and the clipboard bitmap).
#[test]
fn image_equivalence_between_rgba_and_cached_png() {
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-equiv-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let original = rgba(4, 4);
    let result = cache
        .commit(CacheCommitRequest {
            bounds: PhysicalBounds::from_xywh(0, 0, 4, 4),
            size: PhysicalSize::new(4, 4),
            rgba: original.clone(),
            metadata: CacheEntryMetadata::default(),
            monitor_id: "primary".into(),
        })
        .expect("commit");
    // Decode the cached PNG and assert the underlying pixels
    // match the input RGBA buffer. A `png` reader is already in
    // the dependency tree (the cache uses it for `encode_png`), so
    // no new dependency is needed.
    let decoder = png::Decoder::new(std::fs::File::open(&result.entry.png_path).expect("png"));
    let mut reader = decoder.read_info().expect("png info");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).expect("png frame");
    let info = reader.info().clone();
    assert_eq!(info.width, 4);
    assert_eq!(info.height, 4);
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let decoded = &buf[..frame.buffer_size()];
    // The encoder stores the RGBA buffer as-is, so the decoded
    // pixels must equal the input bytes.
    assert_eq!(decoded, original.as_slice());
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn failure_injection_at_write_manifest_reaps_partial() {
    use pixelgrab_contracts::PlatformErrorKind;
    let cache = Cache::new();
    let tmp = std::env::temp_dir().join(format!("pixelgrab-cache-fault-manifest-{}", {
        uuid::Uuid::new_v4().simple()
    }));
    cache.set_cache_root(Some(tmp.clone())).expect("set root");
    let io_err = pixelgrab_contracts::PlatformError::new(
        PlatformErrorKind::Io,
        "injected manifest write failure",
    );
    cache.arm_failure(pixelgrab_lib::cache::CommitStage::WriteManifest, io_err);
    let result = cache.commit(test_request(4, 4));
    assert!(result.is_err(), "WriteManifest failure must abort commit");
    // Phase 2 (manifest) failed: assets exist but no manifest, so the
    // entry is partial. The store reaps it before returning, so the
    // shelf never sees it.
    let entries: Vec<_> = fs::read_dir(&tmp)
        .map(|d| d.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    assert!(
        entries.is_empty(),
        "WriteManifest failure must reap the partial directory, found {:?}",
        entries.iter().map(|e| e.path()).collect::<Vec<_>>()
    );
    fs::remove_dir_all(&tmp).ok();
}
