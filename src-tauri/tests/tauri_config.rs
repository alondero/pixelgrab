//! Regression guard for the Tauri config `plugins` block.
//!
//! Every plugin PixelGrab uses (`single-instance`, `global-shortcut`,
//! `dialog`, `fs`, `clipboard-manager`) is registered programmatically
//! in `lib.rs` via `tauri_plugin_...::init(...)` or
//! `tauri_plugin_...::Builder::new().build()`. None of these plugins
//! takes a JSON-shaped config — their Rust `Config` is the unit type
//! `()`. Declaring any map (even an empty `{}`) makes the plugin's
//! serde deserialiser panic on startup with:
//!
//!     error while running PixelGrab:
//!       PluginInitialization("<plugin>",
//!         "Error deserializing 'plugins.<plugin>' within your Tauri
//!          configuration: invalid type: map, expected unit")
//!
//! That panic happens inside `Builder::run()` after the test suite
//! has finished, so the test suite cannot catch it — only launching
//! the binary does. This regression test guards against any future
//! author copy-pasting `"{plugin}": {}` back into the config.

/// The Tauri config must not declare any `plugins.*` entries.
///
/// See the module docs for the rationale and the panic signature.
#[test]
fn tauri_config_declares_no_plugin_entries() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config_path = manifest_dir.join("tauri.conf.json");
    let raw = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", config_path.display()));
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("tauri.conf.json must be valid JSON");

    let plugins = value
        .get("plugins")
        .and_then(|p| p.as_object())
        .expect("tauri.conf.json must declare a top-level `plugins` object");

    if !plugins.is_empty() {
        let mut keys: Vec<&String> = plugins.keys().collect();
        keys.sort();
        panic!(
            "tauri.conf.json `plugins` block must be empty `{{}}`. \
             Every plugin PixelGrab uses is registered programmatically \
             in lib.rs; declaring a map (even `{{}}`) here makes startup \
             panic with \"invalid type: map, expected unit\" because the \
             plugin Config types are unit-typed. \
             Offending entries: {keys:?}. \
             If a future plugin actually needs config, give it a Rust \
             Config struct that derives Deserialize and accept a JSON map."
        );
    }
}
