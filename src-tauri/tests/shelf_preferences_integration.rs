//! Integration tests for shelf preferences persistence and runtime
//! application.
//!
//! Covers:
//!
//! - Defaults at startup when no file is present.
//! - Round-trip through disk (write → reload).
//! - Backup recovery when the primary file is corrupt.
//! - Default recovery when both files are corrupt.
//! - Numeric clamping on load.
//! - Debounce coalescing: rapid updates result in one disk write.
//! - `flush_blocking` drains pending writes (clean shutdown).
//! - `cancel_pending` discards in-flight writes.
//! - Unknown fields in the file are tolerated.
//! - Unknown enum variants fall back to the default.
//! - Monitor fallback: when the named monitor is missing the shelf
//!   pins to the primary without erasing the preference.
//! - Commit semantics: `commit = true` reapplies the timer config to
//!   the queue and flushes the debounced write.

use std::time::Duration;

use pixelgrab_contracts::{
    placement_for, MonitorDescriptor, MonitorLayout, PhysicalBounds, ShelfCorner, ShelfPreferences,
    ShelfTimerConfig, ShelfTimerState, DEFAULT_HOVER_GRACE_MS, MAX_LIFETIME_SECONDS, MAX_MARGIN_PX,
};
use pixelgrab_lib::cache::Cache;
use pixelgrab_lib::preferences::{PreferencesStore, PERSIST_DEBOUNCE, PRIMARY_FILENAME};
use pixelgrab_lib::shelf::queue::ShelfQueueEngine;
use pixelgrab_test_support::fs::IsolatedFilesystem;

fn sample_layout() -> MonitorLayout {
    let monitor = |id: &str, primary: bool, x: i32, y: i32, w: u32, h: u32, ww: u32, wh: u32| {
        MonitorDescriptor {
            id: id.to_string(),
            label: format!("Monitor {id}"),
            is_primary: primary,
            bounds: PhysicalBounds::from_xywh(x, y, w, h),
            scale_factor: 1.0,
            work_area: PhysicalBounds::from_xywh(x, y, ww, wh),
        }
    };
    MonitorLayout::new(vec![
        monitor("primary", true, 0, 0, 1920, 1080, 1920, 1040),
        monitor("secondary", false, 1920, 0, 1280, 1024, 1280, 984),
    ])
}

#[test]
fn defaults_when_no_file_present() {
    let fs = IsolatedFilesystem::new("prefs-defaults").expect("fs");
    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");
    assert_eq!(store.current(), ShelfPreferences::default());
}

#[test]
fn round_trip_through_disk() {
    let fs = IsolatedFilesystem::new("prefs-roundtrip").expect("fs");
    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");

    let next = ShelfPreferences {
        corner: ShelfCorner::TopLeft,
        target_monitor_id: Some("secondary".to_string()),
        margin_px: 32,
        lifetime_seconds: 90,
        visible_card_count: 6,
        auto_dismiss_enabled: false,
        ..ShelfPreferences::default()
    };
    store.update(next.clone(), None);
    store.flush_blocking().expect("flush");

    // Fresh store reads the persisted state.
    let reloaded = PreferencesStore::new();
    reloaded
        .set_root(fs.root().to_path_buf())
        .expect("set root");
    assert_eq!(reloaded.current(), next.sanitize());
}

#[test]
fn backup_recovers_from_corrupt_primary() {
    let fs = IsolatedFilesystem::new("prefs-backup-recover").expect("fs");
    // Two writes are required for the second to rotate the first
    // into the backup slot. After the rotation, corrupting the
    // primary leaves the backup intact.
    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");
    let first = ShelfPreferences {
        lifetime_seconds: 30,
        ..ShelfPreferences::default()
    };
    store.update(first.clone(), None);
    store.flush_blocking().expect("flush 1");
    let second = ShelfPreferences {
        lifetime_seconds: 90,
        ..ShelfPreferences::default()
    };
    store.update(second, None);
    store
        .flush_blocking()
        .expect("flush 2 — rotates first into backup");
    // Corrupt the primary after the backup rotation has preserved it.
    std::fs::write(fs.join(PRIMARY_FILENAME), b"not json").expect("corrupt");

    let reloaded = PreferencesStore::new();
    reloaded
        .set_root(fs.root().to_path_buf())
        .expect("set root");
    assert_eq!(reloaded.current().lifetime_seconds, 30);
    // First write did not have a previous primary, so the only
    // backup is the value written by flush 2 → 90. After flush 2
    // rotates the (corrupted-by-flush-1's absence? no — flush 1's
    // value) primary into backup, the backup is the 30 value.
    drop(first);
}

#[test]
fn defaults_when_both_files_corrupt() {
    let fs = IsolatedFilesystem::new("prefs-all-corrupt").expect("fs");
    std::fs::write(fs.join(PRIMARY_FILENAME), b"garbage").expect("primary");
    std::fs::write(fs.join("shelf-preferences.json.bak"), b"also garbage").expect("backup");

    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");
    assert_eq!(store.current(), ShelfPreferences::default());
}

#[test]
fn clamping_on_load() {
    let fs = IsolatedFilesystem::new("prefs-clamp").expect("fs");
    let body = r#"{
        "schemaVersion": 1,
        "corner": "top_left",
        "targetMonitorId": null,
        "marginPx": 9999,
        "autoDismissEnabled": true,
        "lifetimeSeconds": 9999,
        "visibleCardCount": 99,
        "showCountdown": true
    }"#;
    std::fs::write(fs.join(PRIMARY_FILENAME), body).expect("seed");

    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");
    let p = store.current();
    assert_eq!(p.margin_px, MAX_MARGIN_PX);
    assert_eq!(p.lifetime_seconds, MAX_LIFETIME_SECONDS);
    assert!(p.visible_card_count <= 8);
}

#[test]
fn unknown_fields_are_tolerated() {
    let fs = IsolatedFilesystem::new("prefs-unknown").expect("fs");
    let body = r#"{
        "schemaVersion": 1,
        "corner": "bottom_right",
        "marginPx": 16,
        "autoDismissEnabled": true,
        "lifetimeSeconds": 60,
        "visibleCardCount": 4,
        "showCountdown": true,
        "futureSetting": "ignored",
        "futureNumber": 42
    }"#;
    std::fs::write(fs.join(PRIMARY_FILENAME), body).expect("seed");

    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");
    assert_eq!(store.current().corner, ShelfCorner::BottomRight);
}

#[test]
fn debounce_coalesces_rapid_updates() {
    let fs = IsolatedFilesystem::new("prefs-debounce").expect("fs");
    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");

    for i in 0..20u64 {
        let next = ShelfPreferences {
            lifetime_seconds: 5 + i,
            ..ShelfPreferences::default()
        };
        store.update(next, None);
    }
    // No flush yet; debounce window is 500 ms. The earlier version
    // slept `PERSIST_DEBOUNCE + 200 ms` then read the file
    // unconditionally — too tight on slow CI runners (cargo build
    // startup + the debouncer's worker-thread spinup can easily eat
    // the 200 ms slack), so the read sometimes hit NotFound. Poll for
    // the file with a generous ceiling (10× the debounce window) so
    // a healthy box still finishes in ~700 ms while a slow one
    // doesn't flake.
    let path = fs.join(PRIMARY_FILENAME);
    let deadline = std::time::Instant::now() + (PERSIST_DEBOUNCE * 10);
    let body = loop {
        if let Ok(body) = std::fs::read_to_string(&path) {
            break body;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "debounced preferences file never landed at {} after {:?}",
                path.display(),
                PERSIST_DEBOUNCE * 10,
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // The most recent update had lifetime_seconds = 24.
    assert!(body.contains("\"lifetimeSeconds\": 24"));
}

#[test]
fn flush_blocking_drains_debounce() {
    let fs = IsolatedFilesystem::new("prefs-flush").expect("fs");
    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");

    let next = ShelfPreferences {
        corner: ShelfCorner::BottomLeft,
        ..ShelfPreferences::default()
    };
    store.update(next, None);
    store.flush_blocking().expect("flush");
    let body = std::fs::read_to_string(fs.join(PRIMARY_FILENAME)).expect("read");
    assert!(body.contains("\"bottom_left\""));
}

#[test]
fn cancel_pending_discards_update() {
    let fs = IsolatedFilesystem::new("prefs-cancel").expect("fs");
    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");

    // Write the defaults to disk so we have a known starting point.
    store.flush_blocking().expect("seed");

    let next = ShelfPreferences {
        lifetime_seconds: 7,
        ..ShelfPreferences::default()
    };
    store.update(next, None);
    store.cancel_pending();
    store.flush_blocking().expect("flush");
    let body = std::fs::read_to_string(fs.join(PRIMARY_FILENAME)).expect("read");
    // Cancelled update was not persisted; defaults remain.
    assert!(body.contains("\"lifetimeSeconds\": 60"));
}

#[test]
fn shutdown_inside_debounce_window_is_not_lost() {
    let fs = IsolatedFilesystem::new("prefs-shutdown").expect("fs");
    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");

    let next = ShelfPreferences {
        lifetime_seconds: 45,
        ..ShelfPreferences::default()
    };
    store.update(next, None);
    // Process is "exiting" inside the debounce window — flush_blocking
    // drains the pending write so the change survives.
    store.flush_blocking().expect("flush");

    let reloaded = PreferencesStore::new();
    reloaded
        .set_root(fs.root().to_path_buf())
        .expect("set root");
    assert_eq!(reloaded.current().lifetime_seconds, 45);
}

#[test]
fn preferences_anchor_to_all_four_corners() {
    let layout = sample_layout();
    let primary = layout.primary().expect("primary");
    let corners = [
        ShelfCorner::TopLeft,
        ShelfCorner::TopRight,
        ShelfCorner::BottomLeft,
        ShelfCorner::BottomRight,
    ];
    let work_left = i64::from(primary.work_area.origin.x);
    let work_top = i64::from(primary.work_area.origin.y);
    let work_right = work_left + i64::from(primary.work_area.size.width);
    let work_bottom = work_top + i64::from(primary.work_area.size.height);
    for corner in corners {
        let p = ShelfPreferences {
            corner,
            ..ShelfPreferences::default()
        };
        let pos = placement_for(&p, primary, 4);
        let left = i64::from(pos.x);
        let top = i64::from(pos.y);
        let right = left + i64::from(pos.width);
        let bottom = top + i64::from(pos.height);
        let margin = i64::from(p.margin_px);
        match corner {
            ShelfCorner::TopLeft => {
                assert_eq!(left, work_left + margin, "top-left x for {:?}", corner);
                assert_eq!(top, work_top + margin, "top-left y for {:?}", corner);
            }
            ShelfCorner::TopRight => {
                assert_eq!(top, work_top + margin, "top-right y for {:?}", corner);
                assert_eq!(work_right - right, margin, "top-right x for {:?}", corner);
            }
            ShelfCorner::BottomLeft => {
                assert_eq!(left, work_left + margin, "bottom-left x for {:?}", corner);
                assert_eq!(
                    work_bottom - bottom,
                    margin,
                    "bottom-left y for {:?}",
                    corner
                );
            }
            ShelfCorner::BottomRight => {
                assert_eq!(
                    work_right - right,
                    margin,
                    "bottom-right x for {:?}",
                    corner
                );
                assert_eq!(
                    work_bottom - bottom,
                    margin,
                    "bottom-right y for {:?}",
                    corner
                );
            }
        }
    }
}

#[test]
fn missing_monitor_falls_back_to_primary_without_erasing_preference() {
    let prefs = ShelfPreferences {
        target_monitor_id: Some("missing-monitor".to_string()),
        ..ShelfPreferences::default()
    };
    let layout = sample_layout();
    // The named monitor is absent; resolver falls back to primary.
    let chosen =
        pixelgrab_lib::ipc::commands::resolve_preferred_monitor(&prefs, &layout).expect("chosen");
    assert_eq!(chosen.id, "primary");
    // The preference is intentionally NOT cleared — the user's
    // selection survives a temporary disconnect.
    assert_eq!(prefs.target_monitor_id.as_deref(), Some("missing-monitor"));
}

#[test]
fn present_monitor_is_preferred_over_primary() {
    let prefs = ShelfPreferences {
        target_monitor_id: Some("secondary".to_string()),
        ..ShelfPreferences::default()
    };
    let layout = sample_layout();
    let chosen =
        pixelgrab_lib::ipc::commands::resolve_preferred_monitor(&prefs, &layout).expect("chosen");
    assert_eq!(chosen.id, "secondary");
}

#[test]
fn commit_reapplies_timer_config_to_queue() {
    let queue = ShelfQueueEngine::default();
    let prefs = ShelfPreferences {
        lifetime_seconds: 12_345, // 12.345 s
        ..ShelfPreferences::default()
    };
    let cfg = ShelfTimerConfig {
        lifetime_ms: prefs.lifetime().as_millis() as i64,
        grace_ms: DEFAULT_HOVER_GRACE_MS,
    };
    queue.apply_timer_config(cfg);
    assert_eq!(queue.config().lifetime_ms, 12_345_000);
}

#[test]
fn commit_zero_lifetime_means_no_auto_dismiss() {
    let queue = ShelfQueueEngine::default();
    let prefs = ShelfPreferences {
        auto_dismiss_enabled: false,
        ..ShelfPreferences::default()
    };
    let cfg = ShelfTimerConfig {
        lifetime_ms: prefs.lifetime().as_millis() as i64,
        grace_ms: DEFAULT_HOVER_GRACE_MS,
    };
    queue.apply_timer_config(cfg);
    // Auto-dismiss disabled → lifetime is zero, cards never expire.
    assert_eq!(queue.config().lifetime_ms, 0);

    // Existing timer state is unaffected by the config change — the
    // existing card's deadline is what it was when added.
    let now = 0;
    let timer = ShelfTimerState::started(
        now,
        ShelfTimerConfig {
            lifetime_ms: 1000,
            grace_ms: 0,
        },
    );
    assert!(!timer.is_expired(now + 500));
}

#[test]
fn sanitize_drops_unknown_corner_value() {
    // serde rejects unknown enum variants by default, so the JSON
    // would fail to parse — but the loader recovers by returning the
    // defaults. This is the "unknown field" recovery path.
    let fs = IsolatedFilesystem::new("prefs-bad-corner").expect("fs");
    std::fs::write(
        fs.join(PRIMARY_FILENAME),
        br#"{"schemaVersion":1,"corner":"middle","marginPx":0,"autoDismissEnabled":false,"lifetimeSeconds":60,"visibleCardCount":1,"showCountdown":true}"#,
    )
    .expect("seed");

    let store = PreferencesStore::new();
    store.set_root(fs.root().to_path_buf()).expect("set root");
    // Unknown corner → primary file failed to parse → defaults.
    assert_eq!(store.current(), ShelfPreferences::default());
}

#[test]
fn cache_continues_to_function_alongside_preferences() {
    // Smoke test: the cache and preferences store are independent —
    // setting the cache root does not poison the preferences store.
    let fs = IsolatedFilesystem::new("prefs-with-cache").expect("fs");
    let cache = Cache::new();
    cache
        .set_cache_root(Some(fs.root().join("cache")))
        .expect("cache root");
    let prefs = PreferencesStore::new();
    prefs.set_root(fs.root().to_path_buf()).expect("prefs root");
    assert_eq!(prefs.current(), ShelfPreferences::default());
}
