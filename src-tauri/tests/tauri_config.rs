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
//!     thread 'main' panicked at src-tauri\src\lib.rs:519:10:
//!     error while building PixelGrab:
//!       PluginInitialization("<plugin>",
//!         "Error deserializing 'plugins.<plugin>' within your Tauri
//!          configuration: invalid type: map, expected unit")
//!
//! Note it is `Builder::build()`, not `Builder::run()` — the panic is the
//! `.expect()` on the `build()` call in `lib.rs::run`. The suite never
//! calls `build()` and never expands `generate_context!()`, so no test
//! reaches this path; only launching the binary does. CI's `Launch smoke
//! test` step in the `e2e` job covers that side. This test covers the
//! static side, guarding against any future author copy-pasting
//! `"{plugin}": {}` back into the config.

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

    // An absent `plugins` key is safer still than an empty one: Tauri reads
    // the block with `config.get(plugin.name()).cloned().unwrap_or_default()`,
    // so a missing key arrives as JSON `null` — which the unit config type
    // accepts. Only a *present* map can break startup, so nothing to check.
    let Some(plugins) = value.get("plugins") else {
        return;
    };
    let plugins = plugins.as_object().unwrap_or_else(|| {
        panic!("`plugins` in tauri.conf.json must be a JSON object, found: {plugins}")
    });

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
