//! Pre-allocated overlay lifecycle. The overlay is built and hidden during
//! application setup so the first capture does not pay a window-creation
//! cost.

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Pre-allocate the overlay window and hide it. Subsequent captures reuse
/// the same window and only toggle visibility.
pub fn preallocate<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window("overlay").is_some() {
        return Ok(());
    }
    let _ = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("overlay.html".into()))
        .title("PixelGrab Overlay")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .resizable(false)
        .build()?;
    Ok(())
}
