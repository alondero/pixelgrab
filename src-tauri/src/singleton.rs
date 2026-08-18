//! Singleton-ownership helpers. The single-instance plugin forwards
//! secondary launches to the running primary process. This module emits the
//! forwarded intent to the existing instance's main window.

use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Bring the existing primary instance to the foreground and emit the
/// forwarded intent. The frontend listens for `pixelgrab://single-instance`.
pub fn forward_to_existing_instance<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = window.emit("pixelgrab://single-instance", ());
    }
}
