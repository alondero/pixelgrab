//! Display-change watcher (issue #63).
//!
//! Windows does not push `WM_DISPLAYCHANGE` into a Tauri webview, so a
//! background thread polls the platform's monitor layout on a fixed
//! interval and compares an order-sensitive fingerprint of every
//! monitor's id, bounds, scale factor, and work area. On change it:
//!
//! 1. invalidates the platform's cached layout (the next capture
//!    re-enumerates instead of using stale geometry);
//! 2. re-anchors any pin whose window escaped the new work-area union;
//! 3. repositions + re-emits the shelf queue snapshot;
//! 4. emits `pixelgrab://display-changed` with the fresh layout so the
//!    frontend can react (settings panel previews, overlay geometry).
//!
//! The reaction body ([`react_to_layout_change`]) is separated from the
//! thread loop so tests can drive it directly.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Runtime};

use crate::preferences::PreferencesStore;
use crate::shelf::queue::ShelfQueueEngine;
use crate::{pin::PinRegistry, platform};

/// The subset of [`crate::PixelGrabApp`] handles the watcher needs.
/// Built before the app state is moved into Tauri's managed storage
/// (`PixelGrabApp` is not `Clone`) and shared with the worker thread.
pub struct DisplayWatchHandles {
    /// Platform contract (layout queries + invalidation).
    pub platform: Arc<dyn platform::PixelGrabPlatform>,
    /// Shelf queue engine.
    pub shelf_queue: Arc<ShelfQueueEngine>,
    /// User shelf preferences.
    pub preferences: Arc<PreferencesStore>,
    /// Pin registry.
    pub pin_registry: Arc<PinRegistry>,
}

/// How often the watcher polls the OS layout. Fast enough that the
/// shelf lands in the right place after a taskbar auto-hide toggle,
/// slow enough to be invisible in the process budget.
pub const DISPLAY_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Spawn the display-change watcher thread. Returns the thread's
/// `JoinHandle`; the binary drops it so the watcher lives for the
/// lifetime of the process.
pub fn spawn_display_watcher<R: Runtime>(
    handles: Arc<DisplayWatchHandles>,
    handle: AppHandle<R>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("pixelgrab-display-watcher".to_string())
        .spawn(move || {
            let previous: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));
            loop {
                std::thread::sleep(DISPLAY_POLL_INTERVAL);
                if let Err(_err) = react_to_layout_change(&handles, &handle, &previous) {
                    // A transient monitor-query failure (e.g. driver
                    // reset mid-poll) is retried on the next tick; the
                    // fingerprint stays untouched so a successful query
                    // after recovery still fires the reaction.
                }
            }
        })
        .expect("spawn display watcher thread")
}

/// Poll the layout once and run the change reactions when the
/// fingerprint differs from the remembered one. Returns `Ok(true)` when
/// a change was detected and reacted to.
///
/// The first call always records the fingerprint without reacting — the
/// startup path has already positioned everything from the boot-time
/// layout, and firing a synthetic "change" at boot would double-emit
/// the shelf event.
pub fn react_to_layout_change<R: tauri::Runtime>(
    handles: &Arc<DisplayWatchHandles>,
    handle: &AppHandle<R>,
    previous: &Mutex<Option<u64>>,
) -> Result<bool, pixelgrab_contracts::PlatformError> {
    // Force a fresh OS query: `monitor_layout` may return a cached
    // layout, which would defeat the whole point of polling.
    handles.platform.invalidate_layout();
    let layout = handles.platform.monitor_layout()?;
    let fingerprint = layout.fingerprint();
    {
        let mut prev = previous.lock();
        match *prev {
            None => {
                *prev = Some(fingerprint);
                return Ok(false);
            }
            Some(prev_fingerprint) if prev_fingerprint == fingerprint => {
                return Ok(false);
            }
            Some(_) => {}
        }
        *prev = Some(fingerprint);
    }

    // 1. Re-anchor pins whose windows escaped the new work areas. Zoom
    //    and opacity are preserved by the registry's re-anchor math.
    if let Some(work_union) = layout.union_work_area() {
        handles.pin_registry.handle_display_change(work_union);
    }

    // 2. Reposition the shelf for the new geometry and emit the fresh
    //    snapshot. An empty queue leaves the shelf hidden — show_queue
    //    no-ops through the same snapshot position logic as commit.
    let snapshot = crate::ipc::commands::snapshot_with_resolved_position(
        &handles.shelf_queue,
        &handles.preferences.current(),
        handles.platform.as_ref(),
    );
    if !snapshot.cards.is_empty() {
        if let Some(position) = snapshot.position.as_ref() {
            if let Err(err) = crate::shelf::show_queue(handle, position) {
                log::warn!("display change: shelf reposition failed: {err}");
            }
        }
    }
    let _ = handle.emit("pixelgrab://shelf-queue-updated", &snapshot);

    // 3. Tell every webview about the new layout so frontend surfaces
    //    (settings preview, overlay hints) can react without polling.
    let _ = handle.emit("pixelgrab://display-changed", &layout);

    // 4. Sync every pin's native window to its (possibly re-anchored)
    //    transform.
    for view in handles.pin_registry.list() {
        if let Err(err) = crate::pin::window::sync_window_to_view(handle, &view) {
            log::warn!("display change: pin window sync failed: {err}");
        }
        crate::pin::window::emit_view(handle, &view);
    }
    Ok(true)
}
