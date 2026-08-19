//! PixelGrab composition root.
//!
//! Wires the platform contract, session state machine, tray, overlay, and
//! Tauri plugins together. Tests drive the public API surface directly to
//! avoid the cost of spinning up a real Tauri runtime.

#![deny(missing_docs)]
#![allow(clippy::needless_return)]

pub mod cache;
pub mod error;
pub mod ipc;
pub mod overlay;
pub mod platform;
pub mod session;
pub mod shelf;
pub mod singleton;
pub mod tray;

#[cfg(any(test, feature = "synthetic"))]
pub use platform::synthetic;

use std::sync::Arc;

use parking_lot::Mutex;
use tauri::Manager;

use crate::session::SessionState;

/// Re-exported types for downstream tests and binaries.
pub use crate::error::PixelGrabError;
pub use crate::session::{EscapeAction, SessionOrchestrator};

/// Application builder used by both the binary and tests.
pub struct PixelGrabApp {
    platform: Arc<dyn platform::PixelGrabPlatform>,
    session: Arc<SessionOrchestrator>,
    cache: Arc<Cache>,
}

impl PixelGrabApp {
    /// Construct a new builder with the given platform implementation.
    pub fn new(platform: Arc<dyn platform::PixelGrabPlatform>) -> Self {
        let session = Arc::new(SessionOrchestrator::new(platform.clone()));
        let cache = Arc::new(Cache::new());
        Self {
            platform,
            session,
            cache,
        }
    }

    /// Handle to the session orchestrator.
    pub fn session(&self) -> Arc<SessionOrchestrator> {
        self.session.clone()
    }

    /// Handle to the platform contract.
    pub fn platform(&self) -> Arc<dyn platform::PixelGrabPlatform> {
        self.platform.clone()
    }

    /// Handle to the cache store.
    pub fn cache(&self) -> Arc<Cache> {
        self.cache.clone()
    }
}

/// Re-export so tests and the IPC layer can name `Cache` directly.
pub use cache::Cache;

/// Run the Tauri application. This is the binary entrypoint.
pub fn run() {
    init_tracing();
    log::info!("PixelGrab starting (tracer-02 capture path)");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            singleton::forward_to_existing_instance(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // Pick the production platform for the current target. The
            // `synthetic` feature is only used by tests and CI - real
            // builds on Windows always run through the Windows adapter
            // and capture a frozen frame before the overlay is revealed.
            let platform: Arc<dyn platform::PixelGrabPlatform> = default_platform();
            let app_state = PixelGrabApp::new(platform);
            app.manage(app_state);

            // Build the resident tray, the hidden overlay window, and
            // the hidden one-card shelf window.
            tray::install(app.handle())?;
            overlay::preallocate(app.handle())?;
            shelf::preallocate(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::request_capture,
            ipc::request_overlay,
            ipc::request_commit,
            ipc::request_cancel,
            ipc::get_session_snapshot,
            ipc::update_cache_metadata,
            ipc::dismiss_cache_entry,
            ipc::get_shelf_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PixelGrab");
}

fn init_tracing() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
}

/// Build the orchestrator at a fresh state for unit tests.
#[cfg(test)]
pub fn test_app() -> PixelGrabApp {
    let platform: Arc<dyn platform::PixelGrabPlatform> =
        Arc::new(platform::synthetic::SyntheticPlatform::new());
    PixelGrabApp::new(platform)
}

/// Pick the production platform for the current target. The Windows
/// adapter is selected on Windows builds; the synthetic adapter is the
/// fallback for non-Windows builds and for CI runs with the `synthetic`
/// feature enabled.
fn default_platform() -> Arc<dyn platform::PixelGrabPlatform> {
    #[cfg(target_os = "windows")]
    {
        if cfg!(feature = "synthetic") {
            return Arc::new(platform::synthetic::SyntheticPlatform::new());
        }
        return Arc::new(platform::windows::WindowsPlatform::new());
    }
    #[cfg(not(target_os = "windows"))]
    {
        Arc::new(platform::synthetic::SyntheticPlatform::new())
    }
}

/// Concise state-machine summary used by tests and the IPC layer.
pub fn summarise_session(app: &PixelGrabApp) -> SessionState {
    app.session().current_state()
}

/// Convenience: expose a parking_lot Mutex wrapper as a `Send`-safe shared.
pub type Shared<T> = Arc<Mutex<T>>;
