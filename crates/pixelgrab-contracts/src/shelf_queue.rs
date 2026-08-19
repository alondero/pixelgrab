//! Shelf queue: the multi-card list of recent captures with per-card timers.
//!
//! Tracer 08 generalises tracer-07's one-card shelf into a queue that
//! shows up to four cards with an expandable `+N` overflow group. Each
//! card carries an independent per-card timer that pauses on hover and
//! resumes with a three-second grace period on mouse leave.
//!
//! The data shapes here are platform-neutral and serialised across the
//! IPC boundary. The queue **engine** that owns the mutable state lives
//! in `src-tauri/src/shelf/queue.rs`; this module only defines the
//! shapes and the per-card timer state machine so it can be exercised
//! without a Tauri runtime.
//!
//! ## Determinism
//!
//! The timer state machine is fully deterministic. All transitions are
//! driven by an injected monotonic clock; "simultaneous" events are
//! processed in shelf-id ascending order so the test suite can assert
//! the exact outcome of every concurrent scenario.

use serde::{Deserialize, Serialize};

use crate::cache::ShelfPosition;

/// Default shelf card lifetime in milliseconds. Picked so that a user
/// has time to read the title and decide whether to keep the capture
/// without feeling rushed.
pub const DEFAULT_CARD_LIFETIME_MS: i64 = 60_000;

/// Default hover grace period in milliseconds. When the mouse leaves a
/// card with less than this much time remaining the deadline is bumped
/// up to `now + grace` so the user never sees a card evaporate because
/// their cursor slipped off the edge.
pub const DEFAULT_HOVER_GRACE_MS: i64 = 3_000;

/// Maximum number of cards rendered in the main shelf row. Older
/// captures move into the expandable overflow group.
pub const MAX_VISIBLE_CARDS: usize = 4;

/// Configuration for the shelf timer behaviour. Exposed so future
/// tracers can override the defaults (e.g. a setting panel) without
/// recomputing every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfTimerConfig {
    /// Total per-card lifetime in milliseconds.
    pub lifetime_ms: i64,
    /// Minimum remaining lifetime when a card is unhovered, in
    /// milliseconds.
    pub grace_ms: i64,
}

impl Default for ShelfTimerConfig {
    fn default() -> Self {
        Self {
            lifetime_ms: DEFAULT_CARD_LIFETIME_MS,
            grace_ms: DEFAULT_HOVER_GRACE_MS,
        }
    }
}

/// Per-card timer state. The fields capture the four interesting points
/// on the timer axis:
///
/// * `added_at_elapsed_ms` — when the card joined the queue.
/// * `deadline_at_elapsed_ms` — when the card should expire.
/// * `paused_at_elapsed_ms` — set when hover begins; `None` while the
///   timer is running.
/// * `paused_remaining_ms` — the remaining time captured at hover
///   time, used to recompute `deadline_at_elapsed_ms` on un-hover.
///
/// The state is serialised so the frontend can render the countdown
/// without re-deriving it from the wall-clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfTimerState {
    /// Monotonic millis when the card joined the queue. Stable for the
    /// lifetime of the card.
    pub added_at_elapsed_ms: i64,
    /// Monotonic millis at which the card expires. Updates when the
    /// card is un-hovered with a grace bump.
    pub deadline_at_elapsed_ms: i64,
    /// When `Some`, the card is paused (hover) and the deadline is
    /// frozen. `None` while the card is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at_elapsed_ms: Option<i64>,
    /// Remaining time captured at hover time. `None` while the card
    /// is running. Combined with `grace_ms` at un-hover to compute
    /// the new deadline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_remaining_ms: Option<i64>,
}

impl ShelfTimerState {
    /// Start a fresh timer for a card joining the queue at `now_ms`.
    pub fn started(now_ms: i64, config: ShelfTimerConfig) -> Self {
        Self {
            added_at_elapsed_ms: now_ms,
            deadline_at_elapsed_ms: now_ms.saturating_add(config.lifetime_ms),
            paused_at_elapsed_ms: None,
            paused_remaining_ms: None,
        }
    }

    /// Mark the card as hovered at `now_ms`. Captures the remaining
    /// time so the un-hover path can re-establish the deadline with
    /// the grace bump. Idempotent: a second `hover` while already
    /// paused is a no-op (and the caller is responsible for treating
    /// it as one — see the engine).
    pub fn hover(&mut self, now_ms: i64) {
        if self.paused_at_elapsed_ms.is_some() {
            return;
        }
        let remaining = self.deadline_at_elapsed_ms.saturating_sub(now_ms).max(0);
        self.paused_at_elapsed_ms = Some(now_ms);
        self.paused_remaining_ms = Some(remaining);
    }

    /// Mark the card as un-hovered at `now_ms`. The new deadline is
    /// `now + max(remaining, grace_ms)`. The grace bump is the only
    /// piece of policy in this module; everything else is mechanical.
    pub fn unhover(&mut self, now_ms: i64, config: ShelfTimerConfig) {
        let remaining = self.paused_remaining_ms.unwrap_or(0);
        let new_remaining = remaining.max(config.grace_ms);
        self.deadline_at_elapsed_ms = now_ms.saturating_add(new_remaining);
        self.paused_at_elapsed_ms = None;
        self.paused_remaining_ms = None;
    }

    /// Remaining milliseconds at `now_ms`. Returns `0` when the card
    /// has already expired. `i64::MAX` is returned if the card is
    /// paused (deadline is frozen); callers that need a finite value
    /// should consult `paused_remaining_ms` directly.
    pub fn remaining_ms(&self, now_ms: i64) -> i64 {
        if self.paused_at_elapsed_ms.is_some() {
            return self.paused_remaining_ms.unwrap_or(0);
        }
        self.deadline_at_elapsed_ms.saturating_sub(now_ms).max(0)
    }

    /// True when the card's deadline has elapsed at `now_ms` and the
    /// card is not currently paused (a paused card cannot expire; the
    /// user is still looking at it).
    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.paused_at_elapsed_ms.is_none() && now_ms >= self.deadline_at_elapsed_ms
    }
}

/// One card on the shelf queue. Combines the durable cache entry view
/// (`ShelfCardView` from `crate::shelf`) with the per-card timer
/// state. Newest-first; the index in `ShelfQueueSnapshot.cards` is the
/// canonical position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfQueueCard {
    /// Shelf card id (UUID v4). Stable for the lifetime of the card.
    pub shelf_id: String,
    /// Capture id (UUID v4) the card represents.
    pub capture_id: String,
    /// Absolute path to the flattened PNG the card displays.
    pub png_path: String,
    /// Total entry size in bytes.
    pub size_bytes: u64,
    /// Wall-clock millis when the entry became durable.
    pub created_at_ms: i64,
    /// Physical bounds of the captured crop.
    pub bounds: crate::coordinate::PhysicalBounds,
    /// Editable metadata persisted with the entry.
    pub metadata: crate::cache::CacheEntryMetadata,
    /// Per-card timer state.
    pub timer: ShelfTimerState,
}

/// Snapshot of the entire shelf queue. Emitted on every commit and on
/// every quick-action event. The frontend is idempotent: re-rendering
/// with the same snapshot produces the same DOM.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfQueueSnapshot {
    /// Visible cards, newest first. Length is at most `MAX_VISIBLE_CARDS`.
    pub cards: Vec<ShelfQueueCard>,
    /// Overflow cards (older than the visible set). Length is the count
    /// of cards hidden behind the `+N` expansion. Order is newest-first
    /// so the expansion reveals cards in the same order they were
    /// committed.
    #[serde(default)]
    pub overflow: Vec<ShelfQueueCard>,
    /// Wall-clock millis the snapshot was computed. The frontend uses
    /// this to interpolate countdown animations between events.
    pub snapshot_at_ms: i64,
    /// Computed placement for the shelf window. Carried alongside the
    /// cards so the frontend can position the webview without a second
    /// IPC call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<ShelfPosition>,
}

impl ShelfQueueSnapshot {
    /// True when the queue holds no cards at all. The frontend hides
    /// the shelf window when this is true.
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty() && self.overflow.is_empty()
    }
}

/// Wire shape for the `copy_shelf_card` IPC. The Rust core reads the
/// PNG bytes from the cache root and publishes them to the system
/// clipboard as PNG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyShelfCardRequest {
    /// Shelf card id whose PNG should be copied to the clipboard.
    pub shelf_id: String,
}

/// Wire shape for the `copy_shelf_card` IPC response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CopyShelfCardResponse {
    /// Number of PNG bytes published. `0` on failure.
    #[serde(default)]
    pub png_bytes: u64,
}

/// Wire shape for the `save_shelf_card_as` IPC. The Rust core opens
/// the native Save As dialog and copies the PNG bytes to the chosen
/// path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveShelfCardAsRequest {
    /// Shelf card id whose PNG should be saved.
    pub shelf_id: String,
}

/// Wire shape for the `save_shelf_card_as` IPC response. `path` is
/// `None` when the user cancelled the dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SaveShelfCardAsResponse {
    /// Absolute path the PNG was written to, or `None` if the dialog
    /// was cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// PNG byte length written. `0` when the dialog was cancelled.
    #[serde(default)]
    pub png_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheEntryMetadata;
    use crate::coordinate::{PhysicalBounds, PhysicalSize};

    fn config() -> ShelfTimerConfig {
        ShelfTimerConfig {
            lifetime_ms: 10_000,
            grace_ms: 1_000,
        }
    }

    #[test]
    fn fresh_timer_has_no_remaining_when_started_at_deadline() {
        let state = ShelfTimerState::started(0, config());
        assert_eq!(state.deadline_at_elapsed_ms, 10_000);
        assert!(state.paused_at_elapsed_ms.is_none());
        assert_eq!(state.remaining_ms(0), 10_000);
    }

    #[test]
    fn hover_freezes_remaining_and_unhover_recomputes_deadline() {
        let mut state = ShelfTimerState::started(0, config());
        // Halfway through.
        state.hover(5_000);
        assert_eq!(state.paused_at_elapsed_ms, Some(5_000));
        assert_eq!(state.paused_remaining_ms, Some(5_000));
        // While paused, the deadline does not advance.
        assert_eq!(state.remaining_ms(9_000), 5_000);
        // Unhover 4 seconds later (paused for 4 s); remaining was 5 s,
        // well above the 1 s grace, so the new deadline is now + 5 s.
        state.unhover(9_000, config());
        assert!(state.paused_at_elapsed_ms.is_none());
        assert_eq!(state.deadline_at_elapsed_ms, 14_000);
    }

    #[test]
    fn unhover_with_small_remaining_bumps_to_grace() {
        let mut state = ShelfTimerState::started(0, config());
        // 9.5 s in — 500 ms remaining.
        state.hover(9_500);
        assert_eq!(state.paused_remaining_ms, Some(500));
        // Unhover 100 ms later — only 500 ms remaining, but grace is
        // 1 s. The new deadline must extend to now + grace.
        state.unhover(9_600, config());
        assert_eq!(state.deadline_at_elapsed_ms, 10_600);
    }

    #[test]
    fn unhover_within_lifetime_does_not_extend_past_original_deadline() {
        // The grace bump is a floor, not a ceiling. If the card had
        // 8 s remaining at hover, the unhover should re-establish
        // the full remaining, not bump it down to grace.
        let mut state = ShelfTimerState::started(0, config());
        state.hover(2_000); // 8 s left
        state.unhover(2_500, config());
        assert_eq!(state.deadline_at_elapsed_ms, 10_500); // now + 8 s
    }

    #[test]
    fn hover_is_idempotent() {
        let mut state = ShelfTimerState::started(0, config());
        state.hover(1_000);
        let snapshot = state.clone();
        state.hover(3_000);
        assert_eq!(state, snapshot);
    }

    #[test]
    fn is_expired_returns_true_only_after_deadline() {
        let mut state = ShelfTimerState::started(0, config());
        assert!(!state.is_expired(9_999));
        assert!(state.is_expired(10_000));
        // Hover freezes the deadline so a hovered card cannot expire.
        state.hover(10_500);
        assert!(!state.is_expired(20_000));
    }

    #[test]
    fn snapshot_is_empty_when_no_cards() {
        let snap = ShelfQueueSnapshot {
            cards: vec![],
            overflow: vec![],
            snapshot_at_ms: 0,
            position: None,
        };
        assert!(snap.is_empty());
    }

    #[test]
    fn snapshot_is_not_empty_when_only_overflow_present() {
        let snap = ShelfQueueSnapshot {
            cards: vec![],
            overflow: vec![ShelfQueueCard {
                shelf_id: "a".into(),
                capture_id: "c".into(),
                png_path: "/tmp/a.png".into(),
                size_bytes: 1,
                created_at_ms: 0,
                bounds: PhysicalBounds::from_xywh(0, 0, 1, 1),
                metadata: CacheEntryMetadata::default(),
                timer: ShelfTimerState::started(0, config()),
            }],
            snapshot_at_ms: 0,
            position: None,
        };
        assert!(!snap.is_empty());
    }

    #[test]
    fn card_timer_state_serialises_to_camel_case() {
        let state = ShelfTimerState::started(0, config());
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"addedAtElapsedMs\""));
        assert!(json.contains("\"deadlineAtElapsedMs\""));
        // Optional fields are skipped while None.
        assert!(!json.contains("\"pausedAtElapsedMs\""));
    }

    #[test]
    fn shelf_queue_snapshot_round_trips() {
        let snap = ShelfQueueSnapshot {
            cards: vec![ShelfQueueCard {
                shelf_id: "shelf".into(),
                capture_id: "cap".into(),
                png_path: "/tmp/c.png".into(),
                size_bytes: 16,
                created_at_ms: 1,
                bounds: PhysicalBounds::from_xywh(0, 0, 4, 4),
                metadata: CacheEntryMetadata::default(),
                timer: ShelfTimerState::started(0, config()),
            }],
            overflow: vec![],
            snapshot_at_ms: 5,
            position: None,
        };
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"snapshotAtMs\":5"));
        assert!(json.contains("\"shelfId\":\"shelf\""));
        let parsed: ShelfQueueSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snap);
        // Size field stays named `size` on PhysicalSize — keep the
        // behaviour stable across the snapshot boundary.
        assert_eq!(parsed.cards[0].bounds.size, PhysicalSize::new(4, 4));
    }
}
