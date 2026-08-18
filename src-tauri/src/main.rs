//! PixelGrab binary entrypoint. The composition root lives in `lib.rs`.

// Prevents additional console window on Windows in release; required for the
// resident tray application.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    pixelgrab_lib::run();
}
