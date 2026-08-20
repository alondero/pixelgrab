//! Shelf window lifecycle. Tracer-07 ships one borderless webview
//! window that floats at the bottom-right of the primary monitor's
//! work area. The window is pre-allocated and repositioned on every
//! successful commit; the same window is shown/hidden by the IPC layer
//! when the shelf card becomes visible or is dismissed.
//!
//! Tracer 08 generalises the one-card shelf into a queue of up to four
//! cards with an expandable `+N` overflow group. The list ordering,
//! per-card timers, hover-pause, and quick actions are owned by
//! [`queue::ShelfQueueEngine`]; the Tauri side of this module only
//! owns the window handle and the position math.
//!
//! The shelf module is intentionally tiny: it owns the Tauri
//! `WebviewWindow` handle and exposes the few helpers the IPC layer
//! needs (`preallocate`, `show_card`, `hide_card`). The actual
//! positioning math lives in `pixelgrab_contracts::ShelfPosition` so
//! the integration tests can exercise it without Tauri.

pub mod queue;

use pixelgrab_contracts::{ShelfId, ShelfPosition};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Pre-allocate the shelf window. Idempotent — early-returns when the
/// window already exists.
pub fn preallocate<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window("shelf").is_some() {
        return Ok(());
    }
    let _ = WebviewWindowBuilder::new(app, "shelf", WebviewUrl::App("shelf.html".into()))
        .title("PixelGrab Shelf")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .resizable(false)
        .focused(false)
        .build()?;
    Ok(())
}

/// Position and show the shelf window at the computed placement. No-op
/// when the window does not exist (e.g. during tests).
pub fn show_card<R: Runtime>(
    app: &AppHandle<R>,
    position: &ShelfPosition,
    _shelf_id: &ShelfId,
) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("shelf") else {
        return Ok(());
    };
    window.set_position(tauri::PhysicalPosition::new(position.x, position.y))?;
    window.set_size(tauri::PhysicalSize::new(position.width, position.height))?;
    window.show()?;
    Ok(())
}

/// Hide the shelf window. No-op when the window does not exist.
pub fn hide_card<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("shelf") else {
        return Ok(());
    };
    window.hide()?;
    Ok(())
}

/// Reposition the shelf window for the given queue snapshot. No-op
/// when the window does not exist or the snapshot is empty. The width
/// of the window scales with the visible card count so all four cards
/// fit side-by-side.
pub fn show_queue<R: Runtime>(app: &AppHandle<R>, position: &ShelfPosition) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("shelf") else {
        return Ok(());
    };
    window.set_position(tauri::PhysicalPosition::new(position.x, position.y))?;
    window.set_size(tauri::PhysicalSize::new(position.width, position.height))?;
    window.show()?;
    Ok(())
}

/// One-card view model for the shelf webview. Sent via the
/// `pixelgrab://shelf-updated` event so the Svelte component can
/// render the most recent card.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfCardView {
    /// Shelf card id (UUID v4).
    pub shelf_id: ShelfId,
    /// Capture id (UUID v4) the card represents.
    pub capture_id: String,
    /// Absolute path to the flattened PNG the card displays.
    pub png_path: String,
    /// Total entry size in bytes.
    pub size_bytes: u64,
    /// Wall-clock millis when the entry became durable.
    pub created_at_ms: i64,
    /// Physical bounds of the captured crop.
    pub bounds: pixelgrab_contracts::PhysicalBounds,
    /// Editable metadata persisted with the entry.
    pub metadata: pixelgrab_contracts::CacheEntryMetadata,
}

impl ShelfCardView {
    /// Build a view from a public `CacheEntry`.
    pub fn from_entry(entry: &pixelgrab_contracts::CacheEntry) -> Self {
        Self {
            shelf_id: entry.shelf_id.clone(),
            capture_id: entry.capture_id.clone(),
            png_path: entry.png_path.clone(),
            size_bytes: entry.size_bytes,
            created_at_ms: entry.created_at_ms,
            bounds: entry.bounds,
            metadata: entry.metadata.clone(),
        }
    }
}

/// Wire payload for the `pixelgrab://shelf-cleared` event. Emitted
/// by `dismiss_cache_entry` once the cache has fully removed the
/// entry (so a listener that does not track the queue snapshot still
/// learns about the removal). The [`ShelfId`] is the same id the
/// `ShelfCardView` carried, so a frontend listener can correlate
/// the cleared event with the queue snapshot that follows it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfClearedEvent {
    /// Shelf id of the entry that was fully removed.
    pub shelf_id: ShelfId,
}
