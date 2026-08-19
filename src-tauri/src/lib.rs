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

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::Manager;

use crate::session::SessionState;
use crate::shelf::queue::ShelfQueueEngine;

/// Re-exported types for downstream tests and binaries.
pub use crate::error::PixelGrabError;
pub use crate::session::{EscapeAction, SessionOrchestrator};

/// Application builder used by both the binary and tests.
pub struct PixelGrabApp {
    platform: Arc<dyn platform::PixelGrabPlatform>,
    session: Arc<SessionOrchestrator>,
    cache: Arc<Cache>,
    /// Shelf queue engine. The queue mirrors the cache for
    /// list-ordering and timer purposes; the cache still owns
    /// durability and the lock registry.
    shelf_queue: Arc<ShelfQueueEngine>,
}

impl PixelGrabApp {
    /// Construct a new builder with the given platform implementation.
    /// The cache root must be set with [`PixelGrabApp::set_cache_root`]
    /// before the cache is used; the Tauri `run` setup hook does this.
    pub fn new(platform: Arc<dyn platform::PixelGrabPlatform>) -> Self {
        let session = Arc::new(SessionOrchestrator::new(platform.clone()));
        let cache = Arc::new(Cache::new());
        let shelf_queue = Arc::new(ShelfQueueEngine::default());
        Self {
            platform,
            session,
            cache,
            shelf_queue,
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

    /// Handle to the shelf queue engine. Tracer 08 moved the
    /// per-card timer state and list ordering out of the cache and
    /// into a dedicated engine so the cache can stay focused on
    /// durability and locks.
    pub fn shelf_queue(&self) -> Arc<ShelfQueueEngine> {
        self.shelf_queue.clone()
    }
}

/// Re-export so tests and the IPC layer can name `Cache` directly.
pub use cache::Cache;

/// Run the Tauri application. This is the binary entrypoint.
pub fn run() {
    init_tracing();
    log::info!("PixelGrab starting (tracer-08 shelf queue path)");

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

            // Wire the cache root and recover any partial entries
            // from a previous run. `load_or_recover` is best-effort:
            // a failed recovery is logged but does not abort startup
            // so the user can still open a capture.
            let cache_root = default_cache_root();
            if let Err(err) = app_state.cache().set_cache_root(Some(cache_root.clone())) {
                log::warn!("cache root {} is unusable: {err}", cache_root.display());
            } else if let Err(err) = app_state.cache().load_or_recover() {
                log::warn!("cache recovery failed: {err}");
            }
            // Rehydrate the queue from the durable cache so cards
            // surviving a process restart remain visible until their
            // timers expire.
            app_state.shelf_queue().rehydrate(
                app_state.cache().entries(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            );
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
            ipc::copy_shelf_card,
            ipc::save_shelf_card_as,
            ipc::hover_shelf_card,
            ipc::unhover_shelf_card,
            ipc::tick_shelf_queue,
            ipc::get_shelf_queue_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PixelGrab");
}

/// Resolve the default on-disk cache root for the current platform.
///
/// On Windows the root lives under `%LOCALAPPDATA%\com.pixelgrab.app\cache`.
/// The folder is created (or reused) by `Cache::set_cache_root`. The
/// directory layout is owned by `cache::store::Cache`; this helper only
/// reports the path.
pub fn default_cache_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local_app_data)
                .join("com.pixelgrab.app")
                .join("cache");
        }
    }
    // Non-Windows (CI, dev on macOS/Linux): fall back to the system
    // temp directory so the cache has a stable, writable home.
    std::env::temp_dir().join("pixelgrab-cache")
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
