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
pub mod pin;
pub mod platform;
pub mod preferences;
pub mod session;
pub mod shelf;
pub mod singleton;
pub mod tray;

#[cfg(any(test, feature = "synthetic"))]
pub use platform::synthetic;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use crate::cache::policy::CachePolicyStore;
use crate::cache::sweeper::{CacheSweeper, SweepWorker};
use crate::pin::PinRegistry;
use crate::preferences::PreferencesStore;
use crate::session::SessionState;
use crate::shelf::queue::ShelfQueueEngine;

/// How often the background shelf ticker expires cards. The frontend
/// also drives its own tick from `requestAnimationFrame` for visual
/// countdowns; this thread is the safety net that guarantees the
/// shelf lock is released even when the webview is hidden,
/// throttled, or has crashed.
pub const SHELF_TICK_INTERVAL: Duration = Duration::from_millis(500);

/// Re-exported types for downstream tests and binaries.
pub use crate::error::PixelGrabError;
pub use crate::pin::{InMemoryPinLockProvider, PinLockGuard};
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
    /// Persistent user shelf preferences (corner, monitor, margins,
    /// timer settings, visible card count, countdown visibility).
    /// Tracer 12. The store owns the in-memory state and debounces
    /// disk writes; `flush_blocking` is called during shutdown.
    preferences: Arc<PreferencesStore>,
    /// Persistent cache policy (max bytes / max entries / max age /
    /// low-water ratio / sweep interval / purge-on-exit).
    /// Tracer 13. Same persistence shape as the shelf preferences.
    cache_policy: Arc<CachePolicyStore>,
    /// Cache sweeper. Tracer 13. Owns the eviction algorithm and
    /// the periodic background worker. The startup recovery sweep
    /// is run on the worker thread so the tray can appear without
    /// waiting for the debris to be reaped.
    sweeper: Arc<CacheSweeper>,
    /// Handle to the periodic background worker. Drop or `stop` to
    /// terminate the worker thread.
    sweep_worker: Mutex<Option<SweepWorker>>,
    /// Pin registry. Tracer 11 ships its own per-pin view model and
    /// cache-lock lifecycle; production wiring will replace the
    /// in-memory lock provider with a shim over the cache's
    /// `ActiveLockSet` so pins and shelf cards share the same lock
    /// registry.
    pin_registry: Arc<PinRegistry>,
}

impl PixelGrabApp {
    /// Construct a new builder with the given platform implementation.
    /// The cache root must be set with [`PixelGrabApp::set_cache_root`]
    /// before the cache is used; the Tauri `run` setup hook does this.
    /// The pin registry uses the in-memory lock provider by default; the
    /// production binary can swap in the shelf's cache lock provider at
    /// setup time (see `lib::run`).
    pub fn new(platform: Arc<dyn platform::PixelGrabPlatform>) -> Self {
        let session = Arc::new(SessionOrchestrator::new(platform.clone()));
        let cache = Arc::new(Cache::new());
        let shelf_queue = Arc::new(ShelfQueueEngine::default());
        let preferences = Arc::new(PreferencesStore::new());
        let cache_policy = Arc::new(CachePolicyStore::new());
        let sweeper = Arc::new(CacheSweeper::new(cache.clone(), cache_policy.clone()));
        let pin_registry = Arc::new(PinRegistry::new(Arc::new(InMemoryPinLockProvider::new())));
        Self {
            platform,
            session,
            cache,
            shelf_queue,
            preferences,
            cache_policy,
            sweeper,
            sweep_worker: Mutex::new(None),
            pin_registry,
        }
    }

    /// Build with a custom pin lock provider. Used by the production
    /// binary when the shelf's cache lock is wired in.
    pub fn with_pin_lock_provider(
        platform: Arc<dyn platform::PixelGrabPlatform>,
        lock_provider: Arc<dyn pixelgrab_contracts::PinLockProvider>,
    ) -> Self {
        let session = Arc::new(SessionOrchestrator::new(platform.clone()));
        let cache = Arc::new(Cache::new());
        let shelf_queue = Arc::new(ShelfQueueEngine::default());
        let preferences = Arc::new(PreferencesStore::new());
        let cache_policy = Arc::new(CachePolicyStore::new());
        let sweeper = Arc::new(CacheSweeper::new(cache.clone(), cache_policy.clone()));
        let pin_registry = Arc::new(PinRegistry::new(lock_provider));
        Self {
            platform,
            session,
            cache,
            shelf_queue,
            preferences,
            cache_policy,
            sweeper,
            sweep_worker: Mutex::new(None),
            pin_registry,
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

    /// Handle to the preferences store. Tracer 12 surfaces user
    /// shelf preferences (corner, monitor, margins, auto-dismiss,
    /// visible-card count, countdown visibility) with debounced
    /// persistence.
    pub fn preferences(&self) -> Arc<PreferencesStore> {
        self.preferences.clone()
    }

    /// Handle to the cache policy store. Tracer 13 surfaces user
    /// cache bounds (max bytes / max entries / max age / low-water
    /// ratio / sweep interval / purge-on-exit) with the same
    /// persistence shape as the shelf preferences.
    pub fn cache_policy(&self) -> Arc<CachePolicyStore> {
        self.cache_policy.clone()
    }

    /// Handle to the cache sweeper. The sweeper owns the eviction
    /// algorithm; the periodic worker is attached via
    /// [`PixelGrabApp::install_sweeper`].
    pub fn sweeper(&self) -> Arc<CacheSweeper> {
        self.sweeper.clone()
    }

    /// Install the periodic background worker. Idempotent: a second
    /// call replaces the existing worker (the previous worker is
    /// stopped first). Safe to call after a policy update so the
    /// new interval takes effect.
    pub fn install_sweeper(&self) {
        let mut guard = self.sweep_worker.lock();
        if let Some(prev) = guard.take() {
            prev.stop();
        }
        let worker = self.sweeper.start_periodic();
        *guard = Some(worker);
    }
}

/// Spawn the background shelf ticker. The thread wakes every
/// `SHELF_TICK_INTERVAL`, expires any cards whose deadline has
/// elapsed, dismisses each from the cache so the shelf lock is
/// released, and re-emits the queue snapshot via the supplied
/// `AppHandle`.
///
/// Takes the individual `Arc` handles (cache, queue, platform)
/// rather than the whole `PixelGrabApp` because `PixelGrabApp` is
/// stored as a Tauri-managed state value (not `Arc`-wrapped) and
/// the thread needs `Send + 'static` handles.
///
/// The function returns the thread's `JoinHandle` for tests; the
/// binary drops it so the ticker lives for the lifetime of the
/// process.
pub fn spawn_shelf_ticker<R: tauri::Runtime>(
    cache: Arc<Cache>,
    queue: Arc<ShelfQueueEngine>,
    platform: Arc<dyn platform::PixelGrabPlatform>,
    preferences: Arc<crate::preferences::PreferencesStore>,
    handle: AppHandle<R>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("pixelgrab-shelf-ticker".to_string())
        .spawn(move || shelf_ticker_loop(cache, queue, platform, preferences, handle))
        .expect("spawn shelf ticker thread")
}

/// Shared ticker loop. Extracted so the binary and any test that
/// wants to advance the engine can both call it without spawning a
/// thread.
fn shelf_ticker_loop<R: tauri::Runtime>(
    cache: Arc<Cache>,
    queue: Arc<ShelfQueueEngine>,
    platform: Arc<dyn platform::PixelGrabPlatform>,
    preferences: Arc<crate::preferences::PreferencesStore>,
    handle: AppHandle<R>,
) {
    let epoch = Instant::now();
    loop {
        std::thread::sleep(SHELF_TICK_INTERVAL);
        let elapsed_ms = epoch.elapsed().as_millis() as i64;
        let outcome = queue.tick(elapsed_ms);
        if outcome.expired.is_empty() {
            continue;
        }
        for shelf_id in &outcome.expired {
            // Privacy: only the shelf id is logged. The cache
            // dismiss error can include the cache path; a stable
            // categorical description is sufficient for telemetry.
            if let Err(_err) = cache.dismiss(shelf_id) {
                log::warn!("shelf_ticker_loop: cache.dismiss failed for shelf_id");
            }
        }
        // Re-emit the new snapshot so the frontend picks up the
        // removal even when the rAF loop was not running.
        let snapshot = {
            let mut snap = queue.snapshot(elapsed_ms);
            let prefs = preferences.current();
            if let Ok(layout) = platform.monitor_layout() {
                if let Some(monitor) =
                    crate::ipc::commands::resolve_preferred_monitor(&prefs, &layout)
                {
                    snap.position = Some(pixelgrab_contracts::placement_for(
                        &prefs,
                        monitor,
                        snap.cards.len(),
                    ));
                }
            }
            snap
        };
        let _ = handle.emit("pixelgrab://shelf-queue-updated", &snapshot);
    }

    /// Handle to the pin registry.
    pub fn pin_registry(&self) -> Arc<PinRegistry> {
        self.pin_registry.clone()
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
            // Wire the preferences root under the same per-app
            // directory as the cache. Both share the `%LOCALAPPDATA%
            // \com.pixelgrab.app` parent; the cache has its own
            // subdirectory so the two cannot collide. The loader
            // tolerates missing / corrupt files (returns defaults),
            // so a first-run install boots cleanly.
            let prefs_root = crate::preferences::default_preferences_root();
            if let Err(err) = app_state.preferences().set_root(prefs_root.clone()) {
                log::warn!(
                    "preferences root {} is unusable: {err}",
                    prefs_root.display()
                );
            }
            // Wire the cache policy store under the same per-app
            // directory. The cache policy is a sibling of the shelf
            // preferences file (`cache-policy.json`), outside the
            // cache root so a partial cache reap cannot delete the
            // user's policy.
            let policy_root = crate::preferences::default_preferences_root();
            if let Err(err) = app_state.cache_policy().set_root(policy_root.clone()) {
                log::warn!(
                    "cache policy root {} is unusable: {err}",
                    policy_root.display()
                );
            }
            // Bootstrap the cache policy file from the defaults when
            // the user has never opened the settings panel. The
            // loader returns the defaults on a missing file so this
            // is a no-op for the common path; the explicit write
            // makes the policy file discoverable on disk for tests.
            app_state.cache_policy().flush_blocking().ok();
            // Non-blocking startup recovery: the sweeper runs on its
            // own thread so the tray can appear without waiting for
            // debris reaping. The periodic worker is installed
            // immediately after.
            let sweeper = app_state.sweeper().clone();
            let cache_for_sweeper = app_state.cache().clone();
            let _recovery_handle = tauri::async_runtime::spawn_blocking(move || {
                if let Err(err) = sweeper.recover_startup() {
                    log::warn!("cache startup recovery failed: {err}");
                }
                // Touch the cache handle so the lifetime is
                // explicit: the spawned closure owns the cache
                // reference for as long as the recovery runs.
                let _ = cache_for_sweeper.stats();
            });
            app_state.install_sweeper();
            // Apply the loaded timer config to the queue so cards
            // added immediately after startup honour the persisted
            // lifetime. A zero lifetime (auto-dismiss off) leaves the
            // existing cards running with their previous deadline —
            // we deliberately don't retroactively expire them, so
            // toggling auto-dismiss on a populated shelf is non-
            // destructive.
            let prefs = app_state.preferences().current();
            let cfg = pixelgrab_contracts::ShelfTimerConfig {
                lifetime_ms: prefs.lifetime().as_millis() as i64,
                grace_ms: pixelgrab_contracts::DEFAULT_HOVER_GRACE_MS,
            };
            app_state.shelf_queue().apply_timer_config(cfg);

            // Rehydrate the queue from the durable cache so cards
            // surviving a process restart remain visible until their
            // timers expire. Uses monotonic millis so a clock change
            // mid-restart cannot expire surviving cards early.
            let rehydrate_ms = crate::ipc::commands::now_ms();
            app_state
                .shelf_queue()
                .rehydrate(app_state.cache().entries(), rehydrate_ms);

            // Borrow the handles we need for the background ticker
            // before `app.manage` consumes `app_state`.
            let ticker_cache = app_state.cache().clone();
            let ticker_queue = app_state.shelf_queue().clone();
            let ticker_platform = app_state.platform().clone();
            let ticker_prefs = app_state.preferences().clone();
            app.manage(app_state);

            // Build the resident tray, the hidden overlay window, and
            // the hidden one-card shelf window.
            tray::install(app.handle())?;
            overlay::preallocate(app.handle())?;
            shelf::preallocate(app.handle())?;

            // Spawn the background shelf ticker so cards expire even
            // when the webview is hidden or throttled. The ticker
            // drives `queue.tick` + `cache.dismiss` per expired id so
            // the shelf lock is always released on time.
            spawn_shelf_ticker(
                ticker_cache,
                ticker_queue,
                ticker_platform,
                ticker_prefs,
                app.handle().clone(),
            );

            // The frontend subscribes to monitor-change events and
            // forwards the new work area to the registry via the
            // `notify_pin_display_change` IPC command. The registry's
            // `handle_display_change` re-anchors orphan pins without
            // resetting zoom or opacity.
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
            ipc::start_shelf_drag,
            ipc::get_shelf_preferences,
            ipc::update_shelf_preferences,
            ipc::open_pin,
            ipc::close_pin,
            ipc::apply_pin_command,
            ipc::get_pin,
            ipc::list_pins,
            ipc::pin_action,
            ipc::notify_pin_display_change,
            ipc::get_cache_policy,
            ipc::update_cache_policy,
            ipc::get_cache_stats,
            ipc::clear_cache,
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
