//! PixelGrab composition root.
//!
//! Wires the platform contract, session state machine, tray, overlay, and
//! Tauri plugins together. Tests drive the public API surface directly to
//! avoid the cost of spinning up a real Tauri runtime.

#![deny(missing_docs)]
#![allow(clippy::needless_return)]

pub mod cache;
pub mod display;
pub mod error;
pub mod hotkey;
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
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry};

use crate::cache::policy::CachePolicyStore;
use crate::cache::sweeper::{CacheSweeper, SweepWorker};
use crate::hotkey::store::HotkeyPreferencesStore;
#[cfg(all(target_os = "windows", not(feature = "synthetic")))]
use crate::hotkey::TauriGlobalShortcutBackend;
use crate::hotkey::{GlobalShortcutBackend, HotkeyRegistry, InMemoryBackend};
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
    /// Pin registry. Every open pin holds a `LockOwner::Pin` reference
    /// on its source entry through the cache's shared lock registry
    /// (`pin::lock::CachePinLockProvider`, issue #63), so the sweeper
    /// and the manual clear cannot evict a pinned PNG while its window
    /// is alive.
    pin_registry: Arc<PinRegistry>,
    /// Hotkey bindings store. Mirrors `PreferencesStore` but
    /// without the debounce so each IPC rebind commits before the
    /// response returns.
    hotkey_store: Arc<HotkeyPreferencesStore>,
    /// Runtime registry that ties the persisted bindings to the
    /// OS-level global shortcut backend. Installed at startup with
    /// the loaded bindings and refreshed on every rebind / pause.
    hotkeys: Arc<HotkeyRegistry>,
}

impl PixelGrabApp {
    /// Construct a new builder with the given platform implementation.
    /// The cache root must be set with [`PixelGrabApp::set_cache_root`]
    /// before the cache is used; the Tauri `run` setup hook does this.
    /// The pin registry is backed by the cache's shared lock registry;
    /// tests that need an isolated provider can use
    /// [`PixelGrabApp::with_pin_lock_provider`].
    ///
    /// The supplied hotkey backend drives the OS-level shortcut
    /// registrations; tests hand in the in-memory fake, the binary
    /// hands in the Tauri global-shortcut adapter. See
    /// `default_hotkey_backend` for the per-build default.
    pub fn new(
        platform: Arc<dyn platform::PixelGrabPlatform>,
        hotkey_backend: Arc<dyn GlobalShortcutBackend>,
    ) -> Self {
        let session = Arc::new(SessionOrchestrator::new(platform.clone()));
        let cache = Arc::new(Cache::new());
        // Issue #63: the pin registry's locks live on the same
        // `ActiveLockSet` the cache uses for shelf / editor / drag
        // ownership, so one registry answers every "is this entry in
        // use?" question.
        let pin_registry = Arc::new(PinRegistry::new(Arc::new(
            crate::pin::lock::CachePinLockProvider::new((*cache).clone()),
        )));
        let shelf_queue = Arc::new(ShelfQueueEngine::default());
        let preferences = Arc::new(PreferencesStore::new());
        let cache_policy = Arc::new(CachePolicyStore::new());
        let sweeper = Arc::new(CacheSweeper::new(cache.clone(), cache_policy.clone()));
        let hotkey_store = Arc::new(HotkeyPreferencesStore::new());
        let hotkeys = Arc::new(HotkeyRegistry::new(hotkey_backend));
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
            hotkey_store,
            hotkeys,
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
        let hotkey_store = Arc::new(HotkeyPreferencesStore::new());
        let hotkeys = Arc::new(HotkeyRegistry::new(InMemoryBackend::new()));
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
            hotkey_store,
            hotkeys,
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

    /// Handle to the hotkey bindings store. Tracer 14 lifts the
    /// shortcut configuration into a parallel JSON document so
    /// rebinds persist across restarts.
    pub fn hotkey_store(&self) -> Arc<HotkeyPreferencesStore> {
        self.hotkey_store.clone()
    }

    /// Handle to the hotkey registry. Holds the in-memory bindings
    /// + the OS registration state.
    pub fn hotkeys(&self) -> Arc<HotkeyRegistry> {
        self.hotkeys.clone()
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

    /// Handle to the pin registry. Pin locks are backed by the cache's
    /// shared lock registry; tests that need an isolated provider can
    /// rebuild the registry via `with_pin_lock_provider`.
    pub fn pin_registry(&self) -> Arc<PinRegistry> {
        self.pin_registry.clone()
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
    loop {
        std::thread::sleep(SHELF_TICK_INTERVAL);
        // Use the exact same process epoch as commit / hover / unhover.
        // A private ticker epoch made restored and newly-added cards live
        // longer by however much the two epoch start times differed.
        let elapsed_ms = crate::ipc::commands::now_ms();
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
        let snapshot = crate::ipc::commands::snapshot_with_resolved_position(
            &queue,
            &preferences.current(),
            platform.as_ref(),
        );
        crate::ipc::commands::sync_shelf_window(&handle, &snapshot);
        let _ = handle.emit("pixelgrab://shelf-queue-updated", &snapshot);
    }
}

/// Re-export so tests and the IPC layer can name `Cache` directly.
pub use cache::Cache;

/// Run the Tauri application. This is the binary entrypoint.
pub fn run() {
    init_tracing();
    log::info!("PixelGrab starting (tracer-14 hotkey + tray path)");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let intent = singleton::parse_launch_intent(&argv);
            singleton::forward_to_existing_instance(app, intent);
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
            // Production Windows builds drive the real
            // `tauri-plugin-global-shortcut` plugin; CI / dev /
            // non-Windows builds stay on the in-memory fake so
            // no OS interaction is required. The plugin must
            // already be initialised when this runs (it is —
            // see the `.plugin(tauri_plugin_global_shortcut::
            // Builder::new().build())` call above), so the
            // `AppHandle` exposes its managed state.
            let hotkey_backend: Arc<dyn GlobalShortcutBackend> =
                install_hotkey_backend(app.handle());
            let app_state = PixelGrabApp::new(platform, hotkey_backend);

            // Wire the cache root and recover any partial entries
            // from a previous run. `load_or_recover` is best-effort:
            // a failed recovery is logged but does not abort startup
            // so the user can still open a capture.
            let cache_root = default_cache_root();
            if let Err(err) = app_state.cache().set_cache_root(Some(cache_root.clone())) {
                // `CacheError::BadRoot` already carries the cache root
                // path in its `Display` payload, so appending the path
                // a second time here would log it twice. The cache root
                // is itself allowed under AGENTS.md §9 (paths outside
                // the cache root are forbidden, the cache root is not).
                log::warn!("{err}");
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
            if app_state
                .preferences()
                .set_root(prefs_root.clone())
                .is_err()
            {
                // The preferences directory is outside the capture cache;
                // log only a category so an absolute user path never leaks.
                log::warn!("preferences root is unusable");
            }
            // Wire the hotkey bindings root under the same parent
            // directory as the shelf preferences. The two JSON
            // documents cannot collide because the filenames are
            // distinct. A corrupt file falls back to defaults.
            let hotkey_root = crate::preferences::default_preferences_root();
            if app_state
                .hotkey_store()
                .set_root(hotkey_root.clone())
                .is_err()
            {
                // The hotkey settings directory is outside the capture
                // cache; preserve the privacy boundary in diagnostics.
                log::warn!("hotkey preferences root is unusable");
            }
            let loaded_bindings = app_state.hotkey_store().current();
            app_state.hotkeys().set_bindings(loaded_bindings.clone());
            let _ = app_state.hotkeys().apply();
            // Wire the cache policy store under the same per-app
            // directory. The cache policy is a sibling of the shelf
            // preferences file (`cache-policy.json`), outside the
            // cache root so a partial cache reap cannot delete the
            // user's policy.
            let policy_root = crate::preferences::default_preferences_root();
            if let Err(_err) = app_state.cache_policy().set_root(policy_root.clone()) {
                // Privacy: AGENTS.md §9 forbids logging paths outside
                // the cache root. The cache policy root is the parent
                // per-app dir, so we log a categorical kind instead.
                log::warn!("cache policy root is unusable");
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
            app_state
                .shelf_queue()
                .apply_visible_card_count(prefs.visible_card_count);

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
            // Issue #63: the display watcher needs the same subset of
            // handles, captured before `manage` consumes `app_state`.
            let watcher_handles = Arc::new(display::DisplayWatchHandles {
                platform: ticker_platform.clone(),
                shelf_queue: ticker_queue.clone(),
                preferences: ticker_prefs.clone(),
                pin_registry: app_state.pin_registry(),
            });
            app.manage(app_state);

            // Build the resident tray, the hidden overlay window, and
            // the hidden one-card shelf window.
            let tray_state = tray::install_with_bindings(app.handle(), &loaded_bindings)?;
            app.manage(tray_state);
            overlay::preallocate(app.handle())?;
            shelf::preallocate(app.handle())?;

            // Rehydration above populated the queue while the native shelf
            // window was still hidden. Synchronize it now so captures that
            // survived a restart are visible without waiting for a new
            // commit or the first timer mutation.
            let startup_snapshot = ipc::snapshot_with_resolved_position(
                &ticker_queue,
                &ticker_prefs.current(),
                ticker_platform.as_ref(),
            );
            ipc::sync_shelf_window(app.handle(), &startup_snapshot);
            let _ = app
                .handle()
                .emit("pixelgrab://shelf-queue-updated", &startup_snapshot);

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

            // Issue #63: poll the OS monitor layout so topology,
            // resolution, DPI, work-area (taskbar), and taskbar changes
            // reposition the shelf and re-anchor pins without a
            // restart. The first tick only records the fingerprint.
            display::spawn_display_watcher(watcher_handles, app.handle().clone());

            // The frontend subscribes to monitor-change events and
            // forwards the new work area to the registry via the
            // `notify_pin_display_change` IPC command. The registry's
            // `handle_display_change` re-anchors orphan pins without
            // resetting zoom or opacity.
            // Honour the user's `purge_on_exit` policy on graceful
            // exit. The cache policy lives on the app state; the
            // shutdown hook (`.run` closure below) consults it and
            // clears unlocked entries before the process terminates.
            // A panic or kill bypasses the hook by design (the spec
            // notes this).
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::request_capture,
            ipc::request_commit,
            ipc::request_cancel,
            ipc::get_session_snapshot,
            ipc::update_cache_metadata,
            ipc::dismiss_cache_entry,
            ipc::get_shelf_snapshot,
            ipc::copy_shelf_card,
            ipc::save_shelf_card_as,
            ipc::save_capture_as,
            ipc::hover_shelf_card,
            ipc::unhover_shelf_card,
            ipc::tick_shelf_queue,
            ipc::get_shelf_queue_snapshot,
            ipc::show_shelf_queue,
            ipc::start_shelf_drag,
            ipc::show_main_window,
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
            ipc::get_hotkey_bindings,
            ipc::update_hotkey_bindings,
            ipc::get_hotkey_status,
            ipc::set_hotkey_paused,
            // Tracer-10: reopen / non-destructive revision flow.
            ipc::open_revision,
            ipc::update_revision,
            ipc::commit_revision,
            ipc::cancel_revision,
        ])
        .build(tauri::generate_context!())
        .expect("error while building PixelGrab")
        .run(handle_run_event);
}

/// Run-event hook for the Tauri app. Implements the teardown
/// ordering required by tracer-14 (hotkeys → tray → preferences
/// flush → cache purge) plus the pre-existing tracer-13 purge
/// policy. The order is important because each step assumes the
/// previous one has completed — e.g. flushing preferences after
/// the tray icon disappears can race the frontend teardown
/// handlers.
fn handle_run_event(app: &AppHandle<tauri::Wry>, event: RunEvent) {
    if let RunEvent::ExitRequested { .. } | RunEvent::Exit = event {
        // 1. Unregister every global shortcut.
        if let Some(state) = app.try_state::<PixelGrabApp>() {
            state.hotkeys().shutdown();
        }
        // 2. Hide the tray icon.
        if let Some(tray) = app.try_state::<crate::tray::TrayState>() {
            tray.shutdown();
        }
        // 3. Force-flush both preference stores. Failures are
        // logged but never abort shutdown so a transient IO error
        // cannot trap the process.
        if let Some(state) = app.try_state::<PixelGrabApp>() {
            if let Err(err) = state.preferences().flush_blocking() {
                log::warn!("shutdown: preferences flush failed: {err:?}");
            }
            if let Err(err) = state.hotkey_store().flush_blocking() {
                log::warn!("shutdown: hotkey bindings flush failed: {err:?}");
            }
            // 4. Honour the user's `purge_on_exit` policy. The cache
            // policy lives on the app state; the shutdown hook
            // consults it and clears unlocked entries before the
            // process terminates. A panic or kill bypasses the
            // hook by design (the spec notes this).
            if state.cache_policy().current().purge_on_exit {
                let _ = state.cache().clear_unlocked_entries();
            }
        }
    }
    if let RunEvent::WindowEvent {
        label,
        event: WindowEvent::CloseRequested { .. },
        ..
    } = &event
    {
        // Hide overlay + shelf windows instead of closing them so
        // the next capture reuses the pre-allocated window rather
        // than spinning up a new one.
        if matches!(label.as_str(), "overlay" | "shelf") {
            if let Some(window) = app.get_webview_window(label) {
                let _ = window.hide();
            }
        }
        // Issue #63: a pin window closed through the OS (Alt+F4,
        // taskbar gesture) must release its registry entry so the
        // cache's `Pin` lock does not leak until restart.
        if let Some(pin_id) = label.strip_prefix("pin-") {
            if let Some(state) = app.try_state::<PixelGrabApp>() {
                let _ = state
                    .pin_registry()
                    .close(&pixelgrab_contracts::PinId::new(pin_id.to_string()));
            }
        }
    }
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

/// Pick a hotkey backend for the running build. Production
/// Windows builds drive the real `tauri-plugin-global-shortcut`
/// plugin; CI / dev / non-Windows builds (and every test that
/// constructs a `PixelGrabApp` directly) stay on the in-memory
/// fake so no OS interaction is required. The plugin's
/// managed state is initialised by the `.plugin(...)` call
/// above the setup hook, so by the time this runs the
/// `AppHandle` already exposes `global_shortcut()`.
fn install_hotkey_backend(handle: &AppHandle<Wry>) -> Arc<dyn GlobalShortcutBackend> {
    #[cfg(all(target_os = "windows", not(feature = "synthetic")))]
    {
        // Production: every shortcut chord lands on the real
        // OS hook list. The handler closures the backend
        // installs emit the existing `pixelgrab://secondary-
        // launch` event so tray clicks, single-instance argv,
        // and chord presses all funnel through one frontend
        // intent handler. The startup log is the surface
        // observers use to confirm the production backend
        // actually got selected — the silent-flip footgun
        // from `default = ["synthetic"]` lived in this branch
        // being skipped without any signal.
        log::info!("hotkey backend: TauriGlobalShortcutBackend (real OS registration)");
        return TauriGlobalShortcutBackend::install(handle.clone());
    }
    #[cfg(any(not(target_os = "windows"), feature = "synthetic"))]
    {
        // CI / dev / non-Windows / tests. Keep the in-memory
        // fake so the registry's transaction semantics can be
        // exercised without involving the OS. Logged loud so a
        // Windows binary built with `--features synthetic`
        // (or a stray `default = ["synthetic"]`) leaves a
        // breadcrumb the user can grep for.
        log::info!(
            "hotkey backend: InMemoryBackend (synthetic — chords will NOT register with the OS)"
        );
        let _ = handle;
        InMemoryBackend::new()
    }
}

/// Build the orchestrator at a fresh state for unit tests.
#[cfg(test)]
pub fn test_app() -> PixelGrabApp {
    let platform: Arc<dyn platform::PixelGrabPlatform> =
        Arc::new(platform::synthetic::SyntheticPlatform::new());
    PixelGrabApp::new(platform, InMemoryBackend::new())
}

/// Pick the production platform for the current target. The Windows
/// adapter is selected on Windows builds; the synthetic adapter is the
/// fallback for non-Windows builds and for CI runs with the `synthetic`
/// feature enabled.
fn default_platform() -> Arc<dyn platform::PixelGrabPlatform> {
    #[cfg(all(target_os = "windows", feature = "synthetic"))]
    {
        log::info!("platform: SyntheticPlatform (synthetic — captures will be deterministic RGBA, not the real desktop)");
        return Arc::new(platform::synthetic::SyntheticPlatform::new());
    }
    #[cfg(all(target_os = "windows", not(feature = "synthetic")))]
    {
        log::info!("platform: WindowsPlatform (real Windows Graphics Capture API)");
        return Arc::new(platform::windows::WindowsPlatform::new());
    }
    #[cfg(not(target_os = "windows"))]
    {
        log::info!("platform: SyntheticPlatform (non-Windows — no real capture path available)");
        Arc::new(platform::synthetic::SyntheticPlatform::new())
    }
}

/// Concise state-machine summary used by tests and the IPC layer.
pub fn summarise_session(app: &PixelGrabApp) -> SessionState {
    app.session().current_state()
}

/// Convenience: expose a parking_lot Mutex wrapper as a `Send`-safe shared.
pub type Shared<T> = Arc<Mutex<T>>;

#[cfg(test)]
mod tests {
    /// Guard against re-introducing `default = ["synthetic"]` in
    /// `src-tauri/Cargo.toml`. That single line used to silently
    /// flip `pnpm tauri:build` / `pnpm tauri:dev` on Windows to
    /// `InMemoryBackend` + `SyntheticPlatform`, so user-installed
    /// binaries never registered chords at the OS layer and the
    /// first symptom was "Ctrl+Shift+S does nothing" (issue
    /// documented in [`pixelgrab-tracer-15`]). The fix flipped the
    /// defaults to `[]` and made CI pass `--features synthetic`
    /// explicitly; this test fails the moment someone re-adds
    /// `synthetic` to the defaults.
    #[test]
    fn default_features_exclude_synthetic() {
        let cargo_toml = include_str!("../Cargo.toml");
        let features_section = cargo_toml
            .split("[features]")
            .nth(1)
            .expect("Cargo.toml must have a [features] section");
        let default_line = features_section
            .lines()
            .map(str::trim_start)
            .find(|line| line.starts_with("default"));
        let Some(default_line) = default_line else {
            // No `default = ...` line at all — Cargo defaults to
            // an empty feature set, which is exactly what we want.
            return;
        };
        assert!(
            !default_line.contains("synthetic"),
            "src-tauri/Cargo.toml [features].default must not include \
             \"synthetic\"; that flag silently flips production Windows \
             builds to InMemoryBackend + SyntheticPlatform so chords \
             never register with the OS. Got: {default_line:?}. See the \
             doc comment on the `synthetic` feature for the rationale."
        );
    }

    /// Pin that the startup-log lines exist on the chosen branch.
    /// The logs are the surface operators grep for when a chord
    /// stops firing, so removing them would erase the only breadcrumb
    /// of the silent-flip regression. We don't assert the exact
    /// wording (free to tweak), only that each branch has at least
    /// one `log::info!` call naming the backend.
    #[test]
    fn install_hotkey_backend_logs_branch_selection() {
        // The function is private; assert via the source text. The
        // string is the compiled artefact so this test will catch a
        // deletion even if the cfg gates move around.
        let src = include_str!("lib.rs");
        let install_start = src
            .find("fn install_hotkey_backend")
            .expect("install_hotkey_backend must exist");
        let install_end = src[install_start..]
            .find("\n}\n")
            .map(|end| install_start + end)
            .expect("install_hotkey_backend must close");
        let body = &src[install_start..install_end];
        assert!(
            body.contains("log::info!"),
            "install_hotkey_backend must emit a log::info! naming which backend got selected; without it a silent-flip regression is invisible. Got body:\n{body}"
        );
    }

    #[test]
    fn default_platform_logs_branch_selection() {
        let src = include_str!("lib.rs");
        let fn_start = src
            .find("fn default_platform")
            .expect("default_platform must exist");
        let fn_end = src[fn_start..]
            .find("\n}\n")
            .map(|end| fn_start + end)
            .expect("default_platform must close");
        let body = &src[fn_start..fn_end];
        assert!(
            body.contains("log::info!"),
            "default_platform must emit a log::info! naming which backend got selected. Got body:\n{body}"
        );
    }
}
