//! Static window-declaration contract tests.
//!
//! Regression guard for the missing `overlay.html` URL (found while
//! validating issue #63): the static `overlay` window in
//! `tauri.conf.json` declared no `url`, so Tauri loaded the default
//! `index.html` into it. Because `overlay::preallocate` early-returns
//! when a window with that label already exists, the real overlay
//! entrypoint was NEVER mounted — pressing the capture hotkey revealed
//! a fullscreen copy of the companion UI with no Konva stage and no
//! region dragging.
//!
//! These tests pin every statically-declared window to its expected
//! entrypoint so the failure mode is caught at test time instead of on
//! a user's desktop.

use serde_json::Value;

fn window_config(label: &str) -> Option<Value> {
    let raw = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"));
    let config: Value = serde_json::from_str(raw).expect("tauri.conf.json parses");
    config["app"]["windows"]
        .as_array()
        .expect("app.windows is an array")
        .iter()
        .find(|w| w["label"].as_str() == Some(label))
        .cloned()
}

#[test]
fn overlay_static_window_declares_the_overlay_entrypoint() {
    let overlay = window_config("overlay").expect("overlay window is declared");
    assert_eq!(
        overlay["url"].as_str(),
        Some("overlay.html"),
        "the static overlay window must load overlay.html; without it \
         preallocate() reuses a window showing index.html and the capture \
         flow has no region selection"
    );
}

#[test]
fn shelf_static_window_declares_the_shelf_entrypoint() {
    let shelf = window_config("shelf").expect("shelf window is declared");
    assert_eq!(shelf["url"].as_str(), Some("shelf.html"));
}

#[test]
fn main_static_window_is_hidden_at_boot() {
    // The companion window starts hidden; the tray is the resident UI.
    let main = window_config("main").expect("main window is declared");
    assert_eq!(main["visible"].as_bool(), Some(false));
}
