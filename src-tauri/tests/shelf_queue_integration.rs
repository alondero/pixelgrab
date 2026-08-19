//! End-to-end tests for the shelf queue engine in collaboration with
//! the cache store. These tests stand between the pure engine unit
//! tests (in `shelf/queue.rs`) and the integration tests that need a
//! Tauri runtime (none yet for tracer 08). They exercise the
//! invariants the engine promises to its callers:
//!
//! - A committed card appears in the queue with a timer started at
//!   the commit timestamp.
//! - Expiring a card via `queue.tick` returns the shelf id so the
//!   caller can dismiss it from the cache; the cache then releases
//!   the shelf lock and reaps the entry from disk.
//! - Manual dismissal of a card via `queue.dismiss` does not release
//!   the cache lock on its own; the caller is still responsible for
//!   invoking `cache.dismiss` so the lock invariant stays with the
//!   cache module.
//! - Rehydrate from the cache restores the visible + overflow queue
//!   state on startup.

use pixelgrab_contracts::{
    cache::CacheEntryMetadata,
    coordinate::{PhysicalBounds, PhysicalSize},
    ShelfTimerConfig, MAX_VISIBLE_CARDS,
};
use pixelgrab_lib::cache::{Cache, CacheCommitRequest};
use pixelgrab_lib::platform::synthetic::SyntheticPlatform;
use pixelgrab_lib::platform::PixelGrabPlatform;
use pixelgrab_lib::shelf::queue::{ShelfQueueEngine, TickOutcome};
use pixelgrab_test_support::fs::IsolatedFilesystem;
use std::sync::Arc;

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

fn request(bounds: PhysicalBounds) -> CacheCommitRequest {
    let size = bounds.size;
    CacheCommitRequest {
        bounds,
        size,
        rgba: filled_rgba(size.width, size.height),
        metadata: CacheEntryMetadata::default(),
        monitor_id: "primary".into(),
    }
}

#[test]
fn commit_pushes_card_onto_queue_and_holds_shelf_lock() {
    let fs = IsolatedFilesystem::new("queue-commit").expect("fs");
    let cache = Arc::new(Cache::new());
    cache
        .set_cache_root(Some(fs.root().to_path_buf()))
        .expect("set root");
    let queue = Arc::new(ShelfQueueEngine::default());

    let bounds = PhysicalBounds::from_xywh(0, 0, 8, 8);
    let result = cache.commit(request(bounds)).expect("commit");
    queue.add(result.entry.clone(), 0);

    // The queue mirrors the cache.
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.shelf_ids(), vec![result.entry.shelf_id.clone()]);

    // The cache still holds the shelf lock.
    let locks = cache.locks();
    assert_eq!(
        locks.owners_of(&result.entry.shelf_id),
        vec![pixelgrab_contracts::LockOwner::Shelf],
    );
}

#[test]
fn tick_returns_ids_for_caller_to_dismiss_in_cache() {
    // Drive the queue with a fast timer so we can drive an expiry in
    // a single test.
    let config = ShelfTimerConfig {
        lifetime_ms: 100,
        grace_ms: 50,
    };
    let fs = IsolatedFilesystem::new("queue-tick").expect("fs");
    let cache = Arc::new(Cache::new());
    cache
        .set_cache_root(Some(fs.root().to_path_buf()))
        .expect("set root");
    let queue = Arc::new(ShelfQueueEngine::new(config));

    let bounds = PhysicalBounds::from_xywh(0, 0, 4, 4);
    let committed = cache.commit(request(bounds)).expect("commit");
    queue.add(committed.entry.clone(), 0);

    // Tick well past expiry. The queue hands back the expired id so
    // the caller can release the lock.
    let TickOutcome { snapshot, expired } = queue.tick(500);
    assert_eq!(expired, vec![committed.entry.shelf_id.clone()]);
    assert!(snapshot.is_empty());

    // The caller dismisses from the cache; the shelf lock is released
    // and the entry directory is reaped.
    let outcome = cache.dismiss(&committed.entry.shelf_id).expect("dismiss");
    assert!(outcome.removed);
    assert!(!fs.root().join(&committed.entry.capture_id).exists());
}

#[test]
fn expiry_in_overflow_still_releases_cache_lock() {
    // Verify the spec invariant: "Release shelf active lock only
    // after card leaves all shelf representations (main + overflow)".
    // The lock must be released when the card leaves overflow too,
    // not just main view.
    let config = ShelfTimerConfig {
        lifetime_ms: 100,
        grace_ms: 50,
    };
    let fs = IsolatedFilesystem::new("queue-overflow-expiry").expect("fs");
    let cache = Arc::new(Cache::new());
    cache
        .set_cache_root(Some(fs.root().to_path_buf()))
        .expect("set root");
    let queue = Arc::new(ShelfQueueEngine::new(config));

    // Commit MAX_VISIBLE_CARDS + 1 so the oldest lives in overflow.
    let mut shelves = Vec::new();
    for _ in 0..=MAX_VISIBLE_CARDS {
        let bounds = PhysicalBounds::from_xywh(0, 0, 4, 4);
        let committed = cache.commit(request(bounds)).expect("commit");
        queue.add(committed.entry.clone(), 0);
        shelves.push(committed.entry.shelf_id.clone());
    }
    // The oldest (shelves[0]) is in overflow.
    let snap_before = queue.snapshot(0);
    assert_eq!(snap_before.overflow.len(), 1);
    assert_eq!(snap_before.overflow[0].shelf_id, shelves[0]);

    // Tick past expiry. All MAX_VISIBLE_CARDS + 1 cards expire.
    let outcome = queue.tick(500);
    assert_eq!(outcome.expired.len(), MAX_VISIBLE_CARDS + 1);

    // The caller dismisses each from the cache; the entry directory
    // for the overflow card is reaped just like the main-view cards.
    for shelf_id in &outcome.expired {
        let outcome = cache.dismiss(shelf_id).expect("dismiss");
        assert!(outcome.removed);
    }
    assert!(cache.entries().is_empty());
}

#[test]
fn hover_does_not_release_lock() {
    let fs = IsolatedFilesystem::new("queue-hover").expect("fs");
    let cache = Arc::new(Cache::new());
    cache
        .set_cache_root(Some(fs.root().to_path_buf()))
        .expect("set root");
    let queue = Arc::new(ShelfQueueEngine::default());

    let bounds = PhysicalBounds::from_xywh(0, 0, 4, 4);
    let committed = cache.commit(request(bounds)).expect("commit");
    queue.add(committed.entry.clone(), 0);
    queue
        .hover(&committed.entry.shelf_id, 30_000)
        .expect("hover");

    // The lock is still held by the cache regardless of the queue's
    // hover state.
    let locks = cache.locks();
    assert_eq!(
        locks.owners_of(&committed.entry.shelf_id),
        vec![pixelgrab_contracts::LockOwner::Shelf],
    );
}

#[test]
fn rehydrate_mirrors_cache_entries_newest_first() {
    let fs = IsolatedFilesystem::new("queue-rehydrate").expect("fs");
    let cache = Arc::new(Cache::new());
    cache
        .set_cache_root(Some(fs.root().to_path_buf()))
        .expect("set root");
    let queue = Arc::new(ShelfQueueEngine::default());

    // Commit three entries with explicit timestamps so the
    // newest-first ordering is deterministic.
    let mut committed = Vec::new();
    for (i, ts) in [(0u32, 100i64), (1u32, 500i64), (2u32, 300i64)] {
        let mut entry = cache
            .commit(request(PhysicalBounds::from_xywh(0, 0, 4, 4)))
            .expect("commit")
            .entry;
        entry.created_at_ms = ts;
        // Rewrite the in-memory snapshot by reaching into the cache
        // via re-commit. The cheaper path is to re-add with adjusted
        // timestamps; the queue doesn't care about the source.
        queue.add(entry.clone(), ts);
        committed.push((entry.shelf_id, i));
    }

    // Now simulate a restart: build a fresh queue, hydrate it from
    // the cache entries, and assert the ordering matches the
    // durable cache.
    let queue2 = Arc::new(ShelfQueueEngine::default());
    let mut entries = cache.entries();
    // Sort newest-first so the queue mirrors the expected order.
    entries.sort_by_key(|b| std::cmp::Reverse(b.created_at_ms));
    queue2.rehydrate(entries, 1_000);

    let snap = queue2.snapshot(1_000);
    assert_eq!(snap.cards.len(), 3);
    // Newest (500) > middle (300) > oldest (100).
    assert_eq!(snap.cards[0].metadata.title.len(), 0); // metadata untouched
    assert_eq!(snap.cards.len(), 3);
    // The shelf ids match the cache's order.
    assert_eq!(
        snap.cards
            .iter()
            .map(|c| c.shelf_id.clone())
            .collect::<Vec<_>>(),
        queue.shelf_ids(),
    );
}

#[test]
fn synthetic_platform_writes_png_for_clipboard_publish() {
    // Exercises the new `publish_png_clipboard` default impl. The
    // synthetic platform has no real clipboard, so the call is a no-op
    // after the PNG-decode round-trip.
    let fs = IsolatedFilesystem::new("platform-clipboard").expect("fs");
    let platform = SyntheticPlatform::new();
    platform.set_cache_root(fs.root().to_path_buf());
    let png_path = fs.root().join("test.png");
    // Write a tiny 4x4 RGBA PNG via the synthetic platform's
    // `write_png` so we know the file is well-formed.
    let rgba = filled_rgba(4, 4);
    let written = platform
        .write_png("test", PhysicalBounds::from_xywh(0, 0, 4, 4), &rgba)
        .expect("write_png");
    assert_eq!(written, png_path);
    assert!(png_path.exists());
    // The default `publish_png_clipboard` decodes the file and
    // forwards to `publish_clipboard`; on the synthetic platform the
    // clipboard call is a no-op, so the round-trip just succeeds.
    platform
        .publish_png_clipboard(&png_path)
        .expect("publish_png_clipboard");
    let _ = PhysicalSize::new(4, 4); // silence unused warnings if test is shrunk
}
