//! Shelf preferences: user-configurable shelf settings with crash-safe
//! persistence.
//!
//! Tracer 12 introduces a versioned settings model so the user can
//! choose:
//!
//! - Which corner of which monitor the shelf anchors to (2x2 picker).
//! - Which monitor hosts the shelf (falls back to primary when the
//!   named monitor disappears).
//! - Margin inset in physical pixels from the work-area edges.
//! - Auto-dismiss toggle + per-card lifetime in seconds.
//! - Number of visible cards (1-8) before overflow kicks in.
//! - Whether the per-card countdown is visible.
//!
//! The on-disk shape is versioned and tolerant of unknown fields so
//! new releases can add fields without breaking older settings files.
//! All numeric fields are clamped to documented ranges; an invalid
//! file is recovered by falling back to [`ShelfPreferences::default`]
//! so the app always boots with a usable configuration.
//!
//! The settings are persisted under
//! `%LOCALAPPDATA%\com.pixelgrab.app\shelf-preferences.json` (next to
//! the cache directory, not inside it). This keeps settings
//! independent of cache reaping — a partial cache reaper must not
//! delete the user's corner choice.
//!
//! See ADR-0007 (shelf preferences persistence) for the rationale.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cache::ShelfPosition;

/// Current schema version. Bump when adding or removing fields. The
/// loader is tolerant of unknown fields, so additive changes don't
/// require a version bump; renaming or removing a field does.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// Default minimum visible-card lifetime in seconds. The lower bound
/// exists so a misconfigured value can't instantly expire cards.
pub const MIN_LIFETIME_SECONDS: u64 = 5;

/// Default maximum visible-card lifetime in seconds. The upper bound
/// keeps a stuck card from sitting on the desktop forever.
pub const MAX_LIFETIME_SECONDS: u64 = 300;

/// Default minimum margin in physical pixels. Smaller than this and
/// the shelf window clips against the work-area edges.
pub const MIN_MARGIN_PX: u32 = 0;

/// Default maximum margin in physical pixels. Larger than this and
/// the shelf window is pushed off the work area on a small monitor.
pub const MAX_MARGIN_PX: u32 = 128;

/// Default minimum number of visible cards. One is the floor.
pub const MIN_VISIBLE_CARDS: u32 = 1;

/// Default maximum number of visible cards. Eight mirrors the
/// tracer-08 overflow cap (which is unbounded, but in practice the
/// UI only renders up to 8 visible cards).
pub const MAX_VISIBLE_CARDS: u32 = 8;

/// Which corner of the chosen monitor the shelf anchors to. The
/// corner is independent of the monitor — the user can keep the
/// primary monitor and put the shelf in the top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShelfCorner {
    /// Top-left of the work area.
    TopLeft,
    /// Top-right of the work area.
    TopRight,
    /// Bottom-left of the work area.
    BottomLeft,
    /// Bottom-right of the work area (the tracer-08 default).
    #[default]
    BottomRight,
}

impl ShelfCorner {
    /// Stable string label for diagnostics and telemetry.
    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "top_left",
            Self::TopRight => "top_right",
            Self::BottomLeft => "bottom_left",
            Self::BottomRight => "bottom_right",
        }
    }
}

/// The user's shelf preferences. Every field is independently clamped
/// or defaulted by [`ShelfPreferences::sanitize`] so an invalid
/// settings file never crashes the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfPreferences {
    /// Schema version the on-disk file was written with. The loader
    /// uses this to branch on older files; future migrations live in
    /// `sanitize`.
    pub schema_version: u32,
    /// Which corner of the work area the shelf anchors to.
    pub corner: ShelfCorner,
    /// Monitor identifier (from `MonitorDescriptor::id`) the shelf
    /// pins to. `None` means "follow the primary monitor".
    #[serde(default)]
    pub target_monitor_id: Option<String>,
    /// Inset from the work-area edges in physical pixels. Clamped to
    /// `[MIN_MARGIN_PX, MAX_MARGIN_PX]`.
    pub margin_px: u32,
    /// Whether cards auto-dismiss after their lifetime. When `false`
    /// the shelf holds cards until the user dismisses them.
    pub auto_dismiss_enabled: bool,
    /// Per-card lifetime in seconds. Clamped to
    /// `[MIN_LIFETIME_SECONDS, MAX_LIFETIME_SECONDS]`.
    pub lifetime_seconds: u64,
    /// Number of cards visible in the main shelf row before overflow
    /// kicks in. Clamped to `[MIN_VISIBLE_CARDS, MAX_VISIBLE_CARDS]`.
    pub visible_card_count: u32,
    /// Whether the per-card countdown text is shown. Hidden when
    /// `false`.
    pub show_countdown: bool,
}

impl Default for ShelfPreferences {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            corner: ShelfCorner::default(),
            target_monitor_id: None,
            margin_px: 24,
            auto_dismiss_enabled: true,
            lifetime_seconds: 60,
            visible_card_count: 4,
            show_countdown: true,
        }
    }
}

impl ShelfPreferences {
    /// Clamp every numeric field to its documented range and reset
    /// out-of-range enums to their default. Called by the loader on
    /// every read so a tampered file can never crash the app.
    pub fn sanitize(&self) -> Self {
        let corner = match self.corner {
            ShelfCorner::TopLeft
            | ShelfCorner::TopRight
            | ShelfCorner::BottomLeft
            | ShelfCorner::BottomRight => self.corner,
        };
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            corner,
            target_monitor_id: self.target_monitor_id.clone(),
            margin_px: self.margin_px.clamp(MIN_MARGIN_PX, MAX_MARGIN_PX),
            auto_dismiss_enabled: self.auto_dismiss_enabled,
            lifetime_seconds: self
                .lifetime_seconds
                .clamp(MIN_LIFETIME_SECONDS, MAX_LIFETIME_SECONDS),
            visible_card_count: self
                .visible_card_count
                .clamp(MIN_VISIBLE_CARDS, MAX_VISIBLE_CARDS),
            show_countdown: self.show_countdown,
        }
    }

    /// Per-card lifetime as a [`Duration`]. Zero when auto-dismiss is
    /// disabled — callers that build a `ShelfTimerConfig` from these
    /// preferences should treat `lifetime.is_zero()` as "no timer".
    pub fn lifetime(&self) -> Duration {
        if self.auto_dismiss_enabled {
            Duration::from_secs(self.lifetime_seconds)
        } else {
            Duration::ZERO
        }
    }

    /// Build a `ShelfTimerConfig` from the preferences. The lifetime
    /// is zero when auto-dismiss is disabled — the queue engine treats
    /// that as "no timer deadline".
    pub fn timer_config(&self) -> ShelfTimerConfigLike {
        ShelfTimerConfigLike {
            lifetime_ms: self.lifetime().as_millis() as i64,
            grace_ms: 3_000,
        }
    }
}

/// A copy of [`crate::shelf_queue::ShelfTimerConfig`] that doesn't
/// force every caller to depend on the shelf_queue module. The queue
/// engine builds the real `ShelfTimerConfig` from this when the
/// preferences change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShelfTimerConfigLike {
    /// Total per-card lifetime in milliseconds. Zero means no timer.
    pub lifetime_ms: i64,
    /// Minimum remaining lifetime when a card is un-hovered, in
    /// milliseconds.
    pub grace_ms: i64,
}

/// Compute the placement for the shelf window given the user
/// preferences and a monitor descriptor. The shelf is anchored to the
/// requested corner of the work area with the configured margin.
pub fn placement_for(
    preferences: &ShelfPreferences,
    monitor: &crate::monitor::MonitorDescriptor,
    visible_cards: usize,
) -> ShelfPosition {
    let count = visible_cards.clamp(1, preferences.visible_card_count as usize);
    let card_width = ShelfPosition::QUEUE_CARD_WIDTH;
    let card_height = ShelfPosition::QUEUE_CARD_HEIGHT;
    let gap = ShelfPosition::QUEUE_CARD_GAP;
    let width = card_width * (count as u32) + gap * ((count as u32).saturating_sub(1));
    let height = card_height;
    let margin = i64::from(preferences.margin_px);
    let work = monitor.work_area;
    let work_left = i64::from(work.origin.x);
    let work_top = i64::from(work.origin.y);
    let work_right = work_left + i64::from(work.size.width);
    let work_bottom = work_top + i64::from(work.size.height);

    let (x, y) = match preferences.corner {
        ShelfCorner::TopLeft => ((work_left + margin) as i32, (work_top + margin) as i32),
        ShelfCorner::TopRight => {
            let right = work_right - margin;
            let x = (right - i64::from(width)).max(work_left + margin) as i32;
            (x, (work_top + margin) as i32)
        }
        ShelfCorner::BottomLeft => (
            (work_left + margin) as i32,
            (work_bottom - margin - i64::from(height)).max(work_top + margin) as i32,
        ),
        ShelfCorner::BottomRight => {
            let right = work_right - margin;
            let bottom = work_bottom - margin;
            let x = (right - i64::from(width)).max(work_left + margin) as i32;
            let y = (bottom - i64::from(height)).max(work_top + margin) as i32;
            (x, y)
        }
    };
    ShelfPosition {
        monitor_id: monitor.id.clone(),
        work_area: work,
        x,
        y,
        width,
        height,
        margin_px: preferences.margin_px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_monitor() -> crate::monitor::MonitorDescriptor {
        use crate::coordinate::PhysicalBounds;
        crate::monitor::MonitorDescriptor {
            id: "primary".to_string(),
            label: "Test Primary".to_string(),
            is_primary: true,
            bounds: PhysicalBounds::from_xywh(0, 0, 1920, 1080),
            scale_factor: 1.0,
            work_area: PhysicalBounds::from_xywh(0, 0, 1920, 1040),
        }
    }

    #[test]
    fn default_matches_tracer_08_baseline() {
        let p = ShelfPreferences::default();
        assert_eq!(p.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(p.corner, ShelfCorner::BottomRight);
        assert!(p.target_monitor_id.is_none());
        assert_eq!(p.margin_px, 24);
        assert!(p.auto_dismiss_enabled);
        assert_eq!(p.lifetime_seconds, 60);
        assert_eq!(p.visible_card_count, 4);
        assert!(p.show_countdown);
    }

    #[test]
    fn sanitize_clamps_out_of_range_numbers() {
        let dirty = ShelfPreferences {
            schema_version: SETTINGS_SCHEMA_VERSION,
            corner: ShelfCorner::BottomRight,
            target_monitor_id: Some("primary".to_string()),
            margin_px: 999,
            auto_dismiss_enabled: true,
            lifetime_seconds: 0,
            visible_card_count: 0,
            show_countdown: true,
        };
        let clean = dirty.sanitize();
        assert_eq!(clean.margin_px, MAX_MARGIN_PX);
        assert_eq!(clean.lifetime_seconds, MIN_LIFETIME_SECONDS);
        assert_eq!(clean.visible_card_count, MIN_VISIBLE_CARDS);
    }

    #[test]
    fn sanitize_resets_unknown_schema_version() {
        let p = ShelfPreferences {
            schema_version: 99,
            ..ShelfPreferences::default()
        };
        let clean = p.sanitize();
        assert_eq!(clean.schema_version, SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn auto_dismiss_disabled_yields_zero_lifetime() {
        let p = ShelfPreferences {
            auto_dismiss_enabled: false,
            ..ShelfPreferences::default()
        };
        assert_eq!(p.lifetime(), Duration::ZERO);
        assert_eq!(p.timer_config().lifetime_ms, 0);
    }

    #[test]
    fn auto_dismiss_enabled_yields_seconds_in_millis() {
        let p = ShelfPreferences::default();
        assert_eq!(
            p.timer_config().lifetime_ms,
            (p.lifetime_seconds as i64) * 1_000
        );
    }

    #[test]
    fn placement_bottom_right_anchors_to_work_area_corner() {
        let p = ShelfPreferences::default();
        let monitor = sample_monitor();
        let pos = placement_for(&p, &monitor, 4);
        let right = i64::from(pos.x) + i64::from(pos.width);
        let wa_right = i64::from(pos.work_area.origin.x) + i64::from(pos.work_area.size.width);
        assert_eq!(wa_right - right, i64::from(p.margin_px));
        let bottom = i64::from(pos.y) + i64::from(pos.height);
        let wa_bottom = i64::from(pos.work_area.origin.y) + i64::from(pos.work_area.size.height);
        assert_eq!(wa_bottom - bottom, i64::from(p.margin_px));
    }

    #[test]
    fn placement_top_left_anchors_to_top_left_corner() {
        let p = ShelfPreferences {
            corner: ShelfCorner::TopLeft,
            ..ShelfPreferences::default()
        };
        let monitor = sample_monitor();
        let pos = placement_for(&p, &monitor, 4);
        assert_eq!(
            i64::from(pos.x),
            i64::from(pos.work_area.origin.x) + i64::from(p.margin_px)
        );
        assert_eq!(
            i64::from(pos.y),
            i64::from(pos.work_area.origin.y) + i64::from(p.margin_px)
        );
    }

    #[test]
    fn placement_top_right_anchors_to_top_right_corner() {
        let p = ShelfPreferences {
            corner: ShelfCorner::TopRight,
            ..ShelfPreferences::default()
        };
        let monitor = sample_monitor();
        let pos = placement_for(&p, &monitor, 4);
        let right = i64::from(pos.x) + i64::from(pos.width);
        let wa_right = i64::from(pos.work_area.origin.x) + i64::from(pos.work_area.size.width);
        assert_eq!(wa_right - right, i64::from(p.margin_px));
        assert_eq!(
            i64::from(pos.y),
            i64::from(pos.work_area.origin.y) + i64::from(p.margin_px)
        );
    }

    #[test]
    fn placement_bottom_left_anchors_to_bottom_left_corner() {
        let p = ShelfPreferences {
            corner: ShelfCorner::BottomLeft,
            ..ShelfPreferences::default()
        };
        let monitor = sample_monitor();
        let pos = placement_for(&p, &monitor, 4);
        let bottom = i64::from(pos.y) + i64::from(pos.height);
        let wa_bottom = i64::from(pos.work_area.origin.y) + i64::from(pos.work_area.size.height);
        assert_eq!(wa_bottom - bottom, i64::from(p.margin_px));
        assert_eq!(
            i64::from(pos.x),
            i64::from(pos.work_area.origin.x) + i64::from(p.margin_px)
        );
    }

    #[test]
    fn placement_respects_visible_card_count_preference() {
        let p = ShelfPreferences {
            visible_card_count: 2,
            ..ShelfPreferences::default()
        };
        let monitor = sample_monitor();
        let pos = placement_for(&p, &monitor, 4);
        // Width should match two cards + one gap, regardless of how many
        // cards the queue actually has right now.
        let expected_width = ShelfPosition::QUEUE_CARD_WIDTH * 2 + ShelfPosition::QUEUE_CARD_GAP;
        assert_eq!(pos.width, expected_width);
    }

    #[test]
    fn placement_clamps_visible_cards_to_preference() {
        let p = ShelfPreferences::default(); // visible_card_count = 4
        let monitor = sample_monitor();
        let pos = placement_for(&p, &monitor, 1);
        let expected_width = ShelfPosition::QUEUE_CARD_WIDTH;
        assert_eq!(pos.width, expected_width);
    }

    #[test]
    fn placement_handles_negative_origin_work_area() {
        let p = ShelfPreferences {
            corner: ShelfCorner::TopRight,
            ..ShelfPreferences::default()
        };
        let mut monitor = sample_monitor();
        monitor.work_area = crate::coordinate::PhysicalBounds::from_xywh(-1920, 0, 1920, 1040);
        monitor.bounds = crate::coordinate::PhysicalBounds::from_xywh(-1920, 0, 1920, 1080);
        let pos = placement_for(&p, &monitor, 4);
        let right = i64::from(pos.x) + i64::from(pos.width);
        let wa_right = i64::from(pos.work_area.origin.x) + i64::from(pos.work_area.size.width);
        assert_eq!(wa_right - right, i64::from(p.margin_px));
        assert!(pos.x >= pos.work_area.origin.x);
    }

    #[test]
    fn corner_labels_are_stable() {
        assert_eq!(ShelfCorner::TopLeft.label(), "top_left");
        assert_eq!(ShelfCorner::TopRight.label(), "top_right");
        assert_eq!(ShelfCorner::BottomLeft.label(), "bottom_left");
        assert_eq!(ShelfCorner::BottomRight.label(), "bottom_right");
    }

    #[test]
    fn round_trip_preserves_fields() {
        let p = ShelfPreferences {
            schema_version: SETTINGS_SCHEMA_VERSION,
            corner: ShelfCorner::TopLeft,
            target_monitor_id: Some("monitor-2".to_string()),
            margin_px: 32,
            auto_dismiss_enabled: false,
            lifetime_seconds: 30,
            visible_card_count: 6,
            show_countdown: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: ShelfPreferences = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let json = r#"{
            "schemaVersion": 1,
            "corner": "top_right",
            "targetMonitorId": null,
            "marginPx": 16,
            "autoDismissEnabled": true,
            "lifetimeSeconds": 90,
            "visibleCardCount": 4,
            "showCountdown": true,
            "futureField": "ignored",
            "futureNumber": 42
        }"#;
        let parsed: ShelfPreferences = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.corner, ShelfCorner::TopRight);
        assert_eq!(parsed.lifetime_seconds, 90);
    }
}
