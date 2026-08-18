//! Resident tray setup. The tracer-01 build installs a minimal menu with
//! "Capture Region" and "Exit" entries; later tracers expand it.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Runtime,
};

/// Install the resident tray icon and its menu. Called from the Tauri setup
/// hook. Returns an error if the icon bytes cannot be decoded.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let capture = MenuItem::with_id(app, "capture_region", "Capture Region", true, None::<&str>)?;
    let full = MenuItem::with_id(
        app,
        "capture_full",
        "Capture Full Screen",
        true,
        None::<&str>,
    )?;
    let shelf = MenuItem::with_id(app, "shelf_history", "Shelf History", true, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        "pause_hotkeys",
        "Pause Global Hotkeys",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&capture, &full, &shelf, &pause, &settings, &sep, &exit],
    )?;

    let icon = placeholder_icon();

    let _ = TrayIconBuilder::with_id("pixelgrab-tray")
        .icon(icon)
        .tooltip("PixelGrab")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "capture_region" => crate::tray::handle_capture_intent(app),
            "capture_full" => crate::tray::handle_capture_intent(app),
            "exit" => app.exit(0),
            _ => log::debug!("tray menu ignored: {:?}", event.id),
        })
        .build(app)?;
    Ok(())
}

/// Emit the capture intent via the Tauri event bus. The frontend listens for
/// `pixelgrab://request-capture` and drives the synthetic capture flow.
pub fn handle_capture_intent<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("pixelgrab://request-capture", ());
    }
}

/// Minimal in-memory RGBA used as the tray icon. The runtime generates a
/// placeholder if the embedded PNG asset is missing.
fn placeholder_icon() -> Image<'static> {
    let size = 32u32;
    let mut buf = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            buf[i] = 0xFF;
            buf[i + 1] = 0xFF;
            buf[i + 2] = 0xFF;
            buf[i + 3] = 0xFF;
        }
    }
    Image::new_owned(buf, size, size)
}
