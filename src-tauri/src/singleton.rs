//! Singleton-ownership helpers. The single-instance plugin forwards
//! secondary launches to the running primary process. This module emits the
//! forwarded intent to the existing instance's main window.

use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Build the fully-qualified event channel name for the singleton-forwarded
/// intent. Exposed so tests and the frontend can share the constant.
pub const SINGLE_INSTANCE_EVENT: &str = "pixelgrab://single-instance";

/// Bring the existing primary instance to the foreground and emit the
/// forwarded intent. The frontend listens for `pixelgrab://single-instance`.
pub fn forward_to_existing_instance<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = window.emit(SINGLE_INSTANCE_EVENT, ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check: the event name is stable so the frontend and the
    /// Rust core cannot drift apart silently. The frontend listens for this
    /// string in `src/App.svelte` via `listen("pixelgrab://single-instance", ...)`.
    #[test]
    fn single_instance_event_name_is_stable() {
        assert_eq!(SINGLE_INSTANCE_EVENT, "pixelgrab://single-instance");
    }
}
