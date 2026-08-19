//! Shelf queue engine. Tracer 08 generalises tracer-07's one-card shelf
//! into a queue that holds up to four visible cards plus an overflow
//! group, with per-card timers that pause on hover and resume with a
//! three-second grace period.
//!
//! The engine owns the mutable queue state behind a single mutex. All
//! transitions are driven by an injected monotonic clock so the test
//! suite can drive every "simultaneous" scenario deterministically. The
//! engine returns a [`ShelfQueueSnapshot`] on every event so the
//! frontend can re-render idempotently.
//!
//! The engine is **not** responsible for releasing shelf locks — that
//! stays with the cache. The engine hands back a list of shelf ids
//! whenever a card is removed from the queue (via expiry or manual
//! dismissal); the caller invokes `cache.dismiss(id)` for each one.
//! This keeps the lock invariant local to the cache module.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::{
    CacheEntry, ShelfId, ShelfQueueCard, ShelfQueueSnapshot, ShelfTimerConfig, ShelfTimerState,
    MAX_VISIBLE_CARDS,
};

/// Outcome of a [`ShelfQueueEngine::tick`] call. The caller is
/// responsible for dismissing each `expired` id via the cache so the
/// shelf lock is released.
#[derive(Debug, Default, Clone)]
pub struct TickOutcome {
    /// New snapshot after the tick (with expired cards removed).
    pub snapshot: ShelfQueueSnapshot,
    /// Shelf ids whose timer elapsed on this tick. The caller
    /// dismisses each one from the cache so the shelf lock is
    /// released and the entry reaped from disk.
    pub expired: Vec<ShelfId>,
}

#[derive(Debug)]
struct QueueEntry {
    entry: CacheEntry,
    timer: ShelfTimerState,
}

#[derive(Debug)]
struct QueueInner {
    /// Cards ordered newest-first. The first `MAX_VISIBLE_CARDS` are
    /// "visible"; the rest are overflow.
    cards: Vec<QueueEntry>,
    /// Lookup index from shelf id to the position in `cards`. Maintained
    /// alongside `cards` so hover/unhover/dismiss are O(1) instead of
    /// scanning the list. The vector is small (≤ ~10 cards in
    /// practice) so a linear scan would also be fine, but a hash map
    /// keeps the code obvious.
    index: HashMap<ShelfId, usize>,
    /// Most recent clock value the engine was driven with. Used by
    /// `snapshot` so the returned `snapshot_at_ms` always matches the
    /// caller's intent even if no event has fired since.
    last_clock_ms: i64,
}

impl QueueInner {
    fn snapshot(&self, now_ms: i64) -> ShelfQueueSnapshot {
        let mut cards = Vec::with_capacity(self.cards.len().min(MAX_VISIBLE_CARDS));
        let mut overflow = Vec::new();
        for (idx, entry) in self.cards.iter().enumerate() {
            let card = ShelfQueueCard {
                shelf_id: entry.entry.shelf_id.clone(),
                capture_id: entry.entry.capture_id.clone(),
                png_path: entry.entry.png_path.clone(),
                size_bytes: entry.entry.size_bytes,
                created_at_ms: entry.entry.created_at_ms,
                bounds: entry.entry.bounds,
                metadata: entry.entry.metadata.clone(),
                timer: entry.timer.clone(),
            };
            if idx < MAX_VISIBLE_CARDS {
                cards.push(card);
            } else {
                overflow.push(card);
            }
        }
        ShelfQueueSnapshot {
            cards,
            overflow,
            snapshot_at_ms: now_ms,
            position: None,
        }
    }

    /// Push a new entry to the front of the list, replacing any prior
    /// copy with the same shelf id. The lookup index is updated to
    /// match.
    fn push_front(&mut self, entry: QueueEntry) {
        if let Some(existing) = self.index.get(&entry.entry.shelf_id).copied() {
            self.cards.remove(existing);
            for slot in self.index.values_mut() {
                if *slot > existing {
                    *slot -= 1;
                }
            }
        }
        self.cards.insert(0, entry);
        // Shift every stored position up by one to account for the
        // insertion at index 0.
        for slot in self.index.values_mut() {
            *slot += 1;
        }
        let shelf_id = self.cards[0].entry.shelf_id.clone();
        self.index.insert(shelf_id, 0);
    }

    /// Remove the entry at `idx` and keep the lookup index consistent.
    fn remove_at(&mut self, idx: usize) -> ShelfId {
        let removed_id = self.cards.remove(idx).entry.shelf_id;
        self.index.remove(&removed_id);
        for slot in self.index.values_mut() {
            if *slot > idx {
                *slot -= 1;
            }
        }
        removed_id
    }
}

/// The shelf queue engine. Cheap to clone (every field is behind an
/// `Arc` or a `Mutex`).
#[derive(Debug, Clone)]
pub struct ShelfQueueEngine {
    inner: Arc<Mutex<QueueInner>>,
    config: ShelfTimerConfig,
}

impl ShelfQueueEngine {
    /// Build a new engine with the given timer configuration.
    pub fn new(config: ShelfTimerConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(QueueInner {
                cards: Vec::new(),
                index: HashMap::new(),
                last_clock_ms: 0,
            })),
            config,
        }
    }

    /// Read the timer configuration.
    pub fn config(&self) -> ShelfTimerConfig {
        self.config
    }

    /// Number of cards currently in the queue (visible + overflow).
    pub fn len(&self) -> usize {
        self.inner.lock().cards.len()
    }

    /// True when the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().cards.is_empty()
    }

    /// Add a new card on commit. The card is inserted at the front
    /// (newest first). If the queue was already at capacity
    /// (`MAX_VISIBLE_CARDS` visible + any overflow), the insertion
    /// bumps the oldest card deeper into the overflow — but does not
    /// remove it. Returns the new snapshot.
    pub fn add(&self, entry: CacheEntry, now_ms: i64) -> ShelfQueueSnapshot {
        let mut inner = self.inner.lock();
        let timer = ShelfTimerState::started(now_ms, self.config);
        inner.push_front(QueueEntry { entry, timer });
        inner.last_clock_ms = now_ms;
        inner.snapshot(now_ms)
    }

    /// Rehydrate the queue from the cache on startup. Existing entries
    /// are added newest-first using `created_at_ms` as the insertion
    /// order; each card's timer is started at `now_ms` so a restart
    /// does not instantly expire pre-existing cards.
    pub fn rehydrate(&self, entries: Vec<CacheEntry>, now_ms: i64) {
        let mut inner = self.inner.lock();
        let mut sorted = entries;
        // Sort oldest-first; we push_front each entry in turn so the
        // last item pushed lands at index 0 = newest. Stable sort so
        // equal timestamps keep their input order, which is
        // deterministic across runs.
        sorted.sort_by_key(|a| a.created_at_ms);
        inner.cards.clear();
        inner.index.clear();
        for entry in sorted {
            let timer = ShelfTimerState::started(now_ms, self.config);
            inner.push_front(QueueEntry { entry, timer });
        }
        inner.last_clock_ms = now_ms;
    }

    /// Mark a card as hovered at `now_ms`. Only the targeted card's
    /// timer pauses; all other cards continue to count down. Returns
    /// `None` if the shelf id is unknown (the frontend should refresh
    /// its snapshot in that case).
    pub fn hover(&self, shelf_id: &str, now_ms: i64) -> Option<ShelfQueueSnapshot> {
        let mut inner = self.inner.lock();
        let idx = *inner.index.get(shelf_id)?;
        let entry = inner.cards.get_mut(idx)?;
        entry.timer.hover(now_ms);
        inner.last_clock_ms = now_ms;
        Some(inner.snapshot(now_ms))
    }

    /// Mark a card as un-hovered at `now_ms`. Resumes the timer with a
    /// grace bump so a card with very little time remaining still gets
    /// at least the configured grace period. Returns `None` if the
    /// shelf id is unknown.
    pub fn unhover(&self, shelf_id: &str, now_ms: i64) -> Option<ShelfQueueSnapshot> {
        let mut inner = self.inner.lock();
        let idx = *inner.index.get(shelf_id)?;
        let entry = inner.cards.get_mut(idx)?;
        entry.timer.unhover(now_ms, self.config);
        inner.last_clock_ms = now_ms;
        Some(inner.snapshot(now_ms))
    }

    /// Tick the queue. Removes any cards whose deadline has elapsed
    /// at `now_ms`. The caller is responsible for dismissing each
    /// returned `expired` id via the cache so the shelf lock is
    /// released.
    ///
    /// Cards in the overflow group expire independently of cards in
    /// the main view — the spec explicitly requires that a card's
    /// "shelf active lock" is only released after the card leaves
    /// **all** shelf representations.
    pub fn tick(&self, now_ms: i64) -> TickOutcome {
        let mut inner = self.inner.lock();
        let mut expired: Vec<ShelfId> = Vec::new();
        // Walk newest-first. Order is deterministic (insertion order)
        // so simultaneous expiry is reproducible.
        let stale_ids: Vec<(usize, ShelfId)> = inner
            .cards
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.timer.is_expired(now_ms))
            .map(|(idx, entry)| (idx, entry.entry.shelf_id.clone()))
            .collect();
        // Remove in reverse so the indices of the trailing entries stay
        // valid as we splice. Iterating the reversed list produces an
        // oldest-first `expired` sequence; the trailing reverse flips
        // it to newest-first so the caller dismisses cards in the
        // same order they disappear from the UI (the most recent
        // capture drops out first).
        for (idx, id) in stale_ids.into_iter().rev() {
            inner.remove_at(idx);
            expired.push(id);
        }
        expired.reverse();
        inner.last_clock_ms = now_ms;
        TickOutcome {
            snapshot: inner.snapshot(now_ms),
            expired,
        }
    }

    /// Manually dismiss a card. Returns `None` if the shelf id is
    /// unknown; the caller is responsible for dismissing the cache
    /// entry to release the shelf lock.
    pub fn dismiss(&self, shelf_id: &str, now_ms: i64) -> Option<ShelfQueueSnapshot> {
        let mut inner = self.inner.lock();
        let idx = *inner.index.get(shelf_id)?;
        inner.remove_at(idx);
        inner.last_clock_ms = now_ms;
        Some(inner.snapshot(now_ms))
    }

    /// Read the current snapshot. `now_ms` is forwarded into the
    /// returned snapshot's `snapshot_at_ms`.
    pub fn snapshot(&self, now_ms: i64) -> ShelfQueueSnapshot {
        let inner = self.inner.lock();
        inner.snapshot(now_ms)
    }

    /// Resolve a card's PNG path. Returns `None` if the shelf id is
    /// not in the queue (the cache is the source of truth for the
    /// PNG; the queue only mirrors it for quick lookups).
    pub fn png_path(&self, shelf_id: &str) -> Option<String> {
        let inner = self.inner.lock();
        let idx = *inner.index.get(shelf_id)?;
        inner.cards.get(idx).map(|e| e.entry.png_path.clone())
    }

    /// All known shelf ids, in newest-first order. Used by the
    /// integration tests to assert the cache and the queue stay in
    /// lockstep.
    pub fn shelf_ids(&self) -> Vec<ShelfId> {
        let inner = self.inner.lock();
        inner
            .cards
            .iter()
            .map(|e| e.entry.shelf_id.clone())
            .collect()
    }
}

impl Default for ShelfQueueEngine {
    fn default() -> Self {
        Self::new(ShelfTimerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelgrab_contracts::cache::CacheEntryMetadata;
    use pixelgrab_contracts::coordinate::{PhysicalBounds, PhysicalSize};

    fn entry(shelf_id: &str, capture_id: &str) -> CacheEntry {
        CacheEntry {
            capture_id: capture_id.to_string(),
            shelf_id: shelf_id.to_string(),
            png_path: format!("/cache/{capture_id}/capture.png"),
            bitmap_path: None,
            bounds: PhysicalBounds::from_xywh(0, 0, 8, 8),
            size: PhysicalSize::new(8, 8),
            size_bytes: 64,
            metadata: CacheEntryMetadata::default(),
            created_at_ms: 0,
            last_access_at_ms: 0,
            monitor_id: "primary".into(),
        }
    }

    fn fast_config() -> ShelfTimerConfig {
        ShelfTimerConfig {
            lifetime_ms: 1_000,
            grace_ms: 100,
        }
    }

    #[test]
    fn new_queue_is_empty() {
        let q = ShelfQueueEngine::default();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn add_inserts_newest_first() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        q.add(entry("b", "cap-b"), 10);
        q.add(entry("c", "cap-c"), 20);
        assert_eq!(q.shelf_ids(), vec!["c", "b", "a"]);
        let snap = q.snapshot(20);
        assert_eq!(snap.cards.len(), 3);
        assert_eq!(snap.overflow.len(), 0);
        assert_eq!(snap.cards[0].shelf_id, "c");
    }

    #[test]
    fn fifth_card_moves_into_overflow() {
        let q = ShelfQueueEngine::new(fast_config());
        for id in ["a", "b", "c", "d", "e"] {
            q.add(entry(id, &format!("cap-{id}")), 0);
        }
        let snap = q.snapshot(0);
        assert_eq!(snap.cards.len(), MAX_VISIBLE_CARDS);
        assert_eq!(snap.overflow.len(), 1);
        assert_eq!(snap.cards[0].shelf_id, "e");
        assert_eq!(snap.overflow[0].shelf_id, "a");
    }

    #[test]
    fn overflow_card_can_be_dismissed_by_id() {
        let q = ShelfQueueEngine::new(fast_config());
        for id in ["a", "b", "c", "d", "e"] {
            q.add(entry(id, &format!("cap-{id}")), 0);
        }
        let snap = q.dismiss("a", 10).expect("dismiss");
        assert_eq!(snap.overflow.len(), 0);
        assert_eq!(snap.cards.len(), MAX_VISIBLE_CARDS);
        assert_eq!(
            snap.cards
                .iter()
                .map(|c| c.shelf_id.clone())
                .collect::<Vec<_>>(),
            vec!["e", "d", "c", "b"],
        );
    }

    #[test]
    fn hover_only_pauses_the_targeted_card() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        q.add(entry("b", "cap-b"), 0);
        let snap = q.hover("a", 900).unwrap();
        let a = snap.cards.iter().find(|c| c.shelf_id == "a").unwrap();
        let b = snap.cards.iter().find(|c| c.shelf_id == "b").unwrap();
        assert!(a.timer.paused_at_elapsed_ms.is_some());
        assert!(b.timer.paused_at_elapsed_ms.is_none());
    }

    #[test]
    fn unhover_resumes_with_grace_when_remaining_is_small() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        let snap_hover = q.hover("a", 950).unwrap();
        assert_eq!(snap_hover.cards[0].timer.paused_remaining_ms, Some(50));
        let snap_unhover = q.unhover("a", 980).unwrap();
        let a = &snap_unhover.cards[0];
        assert_eq!(a.timer.deadline_at_elapsed_ms, 1080);
    }

    #[test]
    fn tick_returns_expired_ids_for_caller_to_dismiss() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        q.add(entry("b", "cap-b"), 0);
        let outcome = q.tick(2_000);
        // Newest-first ordering matches the user-visible
        // disappearance order: the most recently committed card
        // (b) drops out first, then the older one (a).
        assert_eq!(outcome.expired, vec!["b", "a"]);
        assert!(outcome.snapshot.is_empty());
        assert!(q.is_empty());
    }

    #[test]
    fn tick_skips_paused_cards() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        q.hover("a", 500);
        let outcome = q.tick(5_000);
        assert!(outcome.expired.is_empty());
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn tick_processes_overflow_cards() {
        let q = ShelfQueueEngine::new(fast_config());
        for id in ["a", "b", "c", "d", "e"] {
            q.add(entry(id, &format!("cap-{id}")), 0);
        }
        let outcome = q.tick(2_000);
        assert_eq!(outcome.expired, vec!["e", "d", "c", "b", "a"]);
        assert!(outcome.snapshot.is_empty());
    }

    #[test]
    fn hover_unknown_id_returns_none() {
        let q = ShelfQueueEngine::new(fast_config());
        assert!(q.hover("missing", 0).is_none());
        assert!(q.unhover("missing", 0).is_none());
        assert!(q.dismiss("missing", 0).is_none());
    }

    #[test]
    fn dismiss_unknown_id_returns_none() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        assert!(q.dismiss("missing", 10).is_none());
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn add_replaces_existing_card_with_same_shelf_id() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        q.add(entry("b", "cap-b"), 10);
        q.add(entry("a", "cap-a-new"), 20);
        assert_eq!(q.shelf_ids(), vec!["a", "b"]);
    }

    #[test]
    fn rehydrate_orders_by_created_at_desc() {
        let q = ShelfQueueEngine::new(fast_config());
        let mut entries = Vec::new();
        for (id, ts) in [("a", 100), ("b", 500), ("c", 300)] {
            let mut e = entry(id, &format!("cap-{id}"));
            e.created_at_ms = ts;
            entries.push(e);
        }
        q.rehydrate(entries, 1000);
        assert_eq!(q.shelf_ids(), vec!["b", "c", "a"]);
        let outcome = q.tick(1500);
        assert!(outcome.expired.is_empty());
    }

    #[test]
    fn unhover_does_not_reorder_cards() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        q.add(entry("b", "cap-b"), 10);
        q.hover("a", 20);
        let _ = q.unhover("a", 30);
        // Order is unchanged: b first, a second.
        assert_eq!(q.shelf_ids(), vec!["b", "a"]);
    }

    #[test]
    fn dismiss_preserves_paused_state_of_remaining_cards() {
        let q = ShelfQueueEngine::new(fast_config());
        q.add(entry("a", "cap-a"), 0);
        q.add(entry("b", "cap-b"), 10);
        q.hover("b", 100);
        let snap = q.dismiss("a", 100).unwrap();
        // Card b is still paused after a is dismissed.
        let b = snap.cards.iter().find(|c| c.shelf_id == "b").unwrap();
        assert!(b.timer.paused_at_elapsed_ms.is_some());
    }
}
