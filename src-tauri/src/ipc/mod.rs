//! Tauri command handlers. These are the only entry points from the WebView
//! into the Rust core. Each handler returns a typed `IpcResponse`.

pub mod commands;

pub use commands::*;
