//! Native per-pin window lifecycle (issue #63).
//!
//! Every open pin lives in its own borderless, always-on-top,
//! taskbar-hidden webview window labelled `pin-{pinId}`. The window
//! loads the `pin.html` entrypoint with the pin id in the query
//! string; the entrypoint fetches its view model with `get_pin` and
//! renders the `PinWindow` Svelte component — no event race on
//! startup. Subsequent registry updates are pushed as targeted
//! `pixelgrab://pin-viewmodel` events.
//!
//! The registry stays the single source of truth for the transform:
//! after every `apply` / re-anchor the IPC layer calls
//! [`sync_window_to_view`] so the native window tracks the view model.

use pixelgrab_contracts::{PinId, PinViewModel};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Stable window label for a pin. The label doubles as the webview's
/// event target so updates only reach the owning window.
pub fn pin_window_label(pin_id: &PinId) -> String {
    format!("pin-{}", pin_id.as_str())
}

/// Create the native TopMost window for a freshly opened pin and point
/// it at the pin entrypoint with the pin id in the query string. The
/// window is shown immediately — a pin is user-initiated feedback, not
/// background state.
///
/// Returns a categorical error (no paths, no OS internals) when the
/// platform refuses window creation so the caller can roll the
/// registry entry back.
pub fn create_pin_window<R: Runtime>(
    app: &AppHandle<R>,
    view: &PinViewModel,
) -> Result<(), String> {
    let label = pin_window_label(&view.id);
    if app.get_webview_window(&label).is_some() {
        // Stale window from a previous incarnation: reuse it rather
        // than failing the open.
        return sync_window_to_view(app, view);
    }
    let url = format!("pin.html?id={}", view.id.as_str());
    WebviewWindowBuilder::new(app, label.as_str(), WebviewUrl::App(url.into()))
        .title("PixelGrab Pin")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .shadow(false)
        .position(
            view.transform.position.x as f64,
            view.transform.position.y as f64,
        )
        .inner_size(
            view.transform.window_size.width as f64,
            view.transform.window_size.height as f64,
        )
        .build()
        .map_err(|err| format!("pin window creation failed: {err}"))?;
    // The builder only accepts *logical* position/size, which on a
    // scaled monitor would land the window offset from the requested
    // physical transform. Re-apply the exact physical geometry now
    // that the window exists (issue #63 mixed-DPI correctness).
    sync_window_to_view(app, view)?;
    emit_view(app, view);
    Ok(())
}

/// Push the current view model to the pin's webview. Targeted at the
/// owning window so sibling pins do not wake up.
pub fn emit_view<R: Runtime>(app: &AppHandle<R>, view: &PinViewModel) {
    let _ = app.emit_to(
        pin_window_label(&view.id),
        "pixelgrab://pin-viewmodel",
        view,
    );
}

/// Apply the transform's position + size to the native window. No-op
/// when the window is already gone (the close path may race an
/// in-flight zoom command).
pub fn sync_window_to_view<R: Runtime>(
    app: &AppHandle<R>,
    view: &PinViewModel,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window(&pin_window_label(&view.id)) else {
        return Ok(());
    };
    window
        .set_position(tauri::PhysicalPosition::new(
            view.transform.position.x,
            view.transform.position.y,
        ))
        .map_err(|err| format!("pin window move failed: {err}"))?;
    window
        .set_size(tauri::PhysicalSize::new(
            view.transform.window_size.width,
            view.transform.window_size.height,
        ))
        .map_err(|err| format!("pin window resize failed: {err}"))?;
    Ok(())
}

/// Destroy the native window for a closed pin. No-op when the window
/// does not exist.
pub fn destroy_pin_window<R: Runtime>(app: &AppHandle<R>, pin_id: &PinId) {
    if let Some(window) = app.get_webview_window(&pin_window_label(pin_id)) {
        let _ = window.close();
    }
}
