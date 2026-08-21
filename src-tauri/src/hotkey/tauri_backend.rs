//! Production backend that wraps `tauri-plugin-global-shortcut`.
//!
//! Tracer 14 follow-up (issue #46). The previous
//! `default_hotkey_backend()` returned [`crate::hotkey::InMemoryBackend`]
//! for every build target, including production Windows
//! binaries — the OS was never actually contacted. This backend
//! drives the real `tauri_plugin_global_shortcut::GlobalShortcut`
//! plugin so a `cargo build --release` on Windows registers every
//! configured chord at the OS layer.
//!
//! ## Architecture
//!
//! Each `register` call:
//!
//! 1. Parses the canonical binding string (e.g.
//!    `"CommandOrControl+Shift+S"`) into a
//!    [`tauri_plugin_global_shortcut::Shortcut`] using the
//!    `global-hotkey` crate's permissive parser. Our
//!    [`pixelgrab_contracts::parse_binding`] canonical form is a
//!    strict subset of what the upstream parser accepts, so the
//!    two layers never disagree.
//! 2. Calls `app.global_shortcut().on_shortcut(shortcut, handler)`
//!    to install the OS-level handler. The plugin performs the
//!    actual registration on the main thread; a backend rejection
//!    is reported back through the channel that the plugin
//!    uses to bridge main-thread and caller.
//! 3. Inserts the new `(Shortcut, canonical)` pair into the
//!    in-memory state and the `shortcut.id() -> HotkeyAction`
//!    lookup table.
//!
//! The handler closure captures an `Arc<Mutex<SharedState>>` so
//! the per-shortcut closures can resolve which
//! [`HotkeyAction`] was pressed (and emit the matching
//! `pixelgrab://secondary-launch` event) without re-allocating
//! per call.
//!
//! ## Single source of truth
//!
//! The handler emits the same `pixelgrab://secondary-launch` event
//! the tray menu uses (see [`crate::singleton::SINGLE_INSTANCE_EVENT`]),
//! so the frontend listener routes tray clicks, global shortcuts,
//! and secondary-launch argv through one intent dispatcher. See
//! `crate::tray::forward_intent` for the matching tray path.
//!
//! ## Lifecycle
//!
//! The backend is installed from `lib::run` after the
//! `tauri_plugin_global_shortcut` plugin has been initialised.
//! The plugin stores its state under the `AppHandle`'s managed
//! state; [`install`](Self::install) captures the `AppHandle` and
//! returns a backend whose [`GlobalShortcutBackend::register`]
//! implementation calls `app.global_shortcut().on_shortcut(...)`.
//! The in-memory backend continues to be used by every test and
//! by the `synthetic` feature on non-Windows hosts.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use pixelgrab_contracts::{HotkeyAction, PlatformError, PlatformErrorKind, SecondaryLaunchIntent};

use crate::hotkey::GlobalShortcutBackend;
use crate::singleton::SINGLE_INSTANCE_EVENT;

/// Production backend that drives
/// `tauri_plugin_global_shortcut::GlobalShortcut`. Cheap to
/// clone — the entire state lives inside an `Arc<Mutex<...>>`.
#[derive(Debug, Clone)]
pub struct TauriGlobalShortcutBackend {
    state: Arc<Mutex<SharedState>>,
}

/// Per-process state shared between the backend's bookkeeping
/// methods and the per-shortcut handler closures. Holding the
/// `AppHandle` here lets the handler emit the secondary-launch
/// event without borrowing from `app.global_shortcut()`.
#[derive(Debug)]
struct SharedState {
    /// Tauri app handle. `None` until [`install`] runs; every
    /// method asserts it's set because the backend is only
    /// constructed through [`install`] in production.
    handle: Option<AppHandle<Wry>>,
    /// `(Shortcut, canonical string)` per registered action.
    /// `canonical` is the post-`parse_binding` form the
    /// registry handed us; it's the value
    /// [`currently_registered`](GlobalShortcutBackend::currently_registered)
    /// returns to callers that need to verify the OS view.
    registered: HashMap<HotkeyAction, RegisteredShortcut>,
    /// `shortcut.id() -> HotkeyAction` so the per-shortcut
    /// handler closure can resolve which action fired in O(1).
    /// Mirrors `registered` by id; the two maps are always
    /// updated together so they cannot drift.
    id_to_action: HashMap<u32, HotkeyAction>,
}

#[derive(Debug, Clone, Copy)]
struct RegisteredShortcut {
    shortcut: Shortcut,
    /// Canonical binding string the registry handed us. Held
    /// across calls so [`currently_registered`](GlobalShortcutBackend::currently_registered)
    /// can return the same form the OS sees — the registry's
    /// input is `&str` borrowed from an IPC payload, so we
    /// intern it via [`intern_canonical`] to get a `'static` ref.
    canonical: &'static str,
}

/// Intern canonical strings into a process-lifetime table. The
/// registry hands us `&str` borrowed from a `HotkeyBindings`
/// payload that lives at most as long as the IPC command, so the
/// backend needs a `'static` ref to keep across calls. The
/// `Mutex<HashMap>` dedups by content so a hot rebind to the
/// same chord re-uses the same interned slice — a per-call
/// `Box::leak` would leak one copy per rebind instead.
fn intern_canonical(canonical: &str) -> &'static str {
    use std::sync::OnceLock;
    static TABLE: OnceLock<parking_lot::Mutex<std::collections::HashMap<String, &'static str>>> =
        OnceLock::new();
    let table = TABLE.get_or_init(|| parking_lot::Mutex::new(std::collections::HashMap::new()));
    let mut guard = table.lock();
    if let Some(existing) = guard.get(canonical) {
        return existing;
    }
    let leaked: &'static str = Box::leak(canonical.to_owned().into_boxed_str());
    guard.insert(leaked.to_string(), leaked);
    leaked
}

impl TauriGlobalShortcutBackend {
    /// Build a backend wired to the running `AppHandle`. Must be
    /// called from the Tauri setup hook so the global-shortcut
    /// plugin is already initialised under the handle.
    pub fn install(handle: AppHandle<Wry>) -> Arc<Self> {
        // Pre-flight check: the plugin's managed state must be
        // retrievable before we stash a copy. A panic here
        // surfaces a misconfigured plugin chain (e.g.
        // `Builder::new().build()` missing from the Tauri
        // builder) before the first IPC call.
        handle.global_shortcut();
        Arc::new(Self {
            state: Arc::new(Mutex::new(SharedState {
                handle: Some(handle),
                registered: HashMap::new(),
                id_to_action: HashMap::new(),
            })),
        })
    }

    /// Parse a canonical binding string into a plugin-friendly
    /// `Shortcut`. Public so the integration tests can drive the
    /// parser without spinning up a Tauri runtime.
    pub fn parse_shortcut(canonical: &str) -> Result<Shortcut, PlatformError> {
        // Privacy: the upstream `global-hotkey` error Display can
        // include user-supplied paths on some platforms. AGENTS.md
        // §9 forbids leaking anything outside the cache root
        // through the IPC boundary, so we report a categorical
        // kind + the canonical string length (no raw text).
        canonical.parse::<Shortcut>().map_err(|_err| {
            PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("invalid_shortcut[len={}]", canonical.len()),
            )
        })
    }
}

impl GlobalShortcutBackend for TauriGlobalShortcutBackend {
    fn register(&self, action: HotkeyAction, canonical: &str) -> Result<(), PlatformError> {
        let shortcut = Self::parse_shortcut(canonical)?;
        let mut guard = self.state.lock();
        let handle = guard.handle.clone().ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::Internal,
                "TauriGlobalShortcutBackend::register before install",
            )
        })?;

        // If the same action is already registered with the
        // exact same canonical string, the call is idempotent —
        // the plugin's `on_shortcut` will overwrite the handler
        // but leave the OS handle intact (the id is stable for a
        // given `(mods, key)` pair).
        if let Some(existing) = guard.registered.get(&action) {
            if existing.shortcut.id() == shortcut.id() {
                return Ok(());
            }
        }

        // Build the per-shortcut handler closure. It captures
        // the shared state so a press on any registered chord
        // can resolve which `HotkeyAction` fired and emit the
        // matching intent on the existing event channel.
        let state = self.state.clone();
        let handler = move |_app: &AppHandle<Wry>, shortcut: &Shortcut, event: ShortcutEvent| {
            // Use eprintln! (not log::info!) — log strings get
            // stripped by release-mode LTO when the closure is
            // behind a `Box<dyn Fn>`. eprintln! keeps the format
            // string in the binary so the diagnostic survives.
            eprintln!(
                "[HOTKEY] handler fired: id={} state={:?}",
                shortcut.id(),
                event.state()
            );
            if event.state() != ShortcutState::Pressed {
                // Ignore the release event; the press is what
                // the user perceives as "the shortcut fired".
                return;
            }
            let (action, handle) = {
                let s = state.lock();
                let action = s.id_to_action.get(&shortcut.id()).copied();
                let handle = s.handle.clone();
                (action, handle)
            };
            if let (Some(action), Some(handle)) = (action, handle) {
                eprintln!("[HOTKEY] resolving to action {:?}", action);
                emit_secondary_launch(&handle, action);
            } else {
                eprintln!("[HOTKEY] no action/handle for id={}", shortcut.id());
            }
        };

        // `on_shortcut` blocks on the main-thread hop that the
        // plugin performs internally; the IPC caller pays the
        // cost. The plugin's `register_internal` rolls back the
        // OS registration if the manager rejects the chord, so a
        // backend rejection surfaces here as an Err and we leave
        // our maps untouched.
        //
        // Privacy: same as `parse_shortcut` — the plugin's error
        // Display is categorically scrubbed before crossing the
        // IPC boundary (AGENTS.md §9).
        handle
            .global_shortcut()
            .on_shortcut(shortcut, handler)
            .map_err(|_err| {
                PlatformError::new(PlatformErrorKind::Internal, "on_shortcut_failed")
            })?;

        // If the action was previously bound to a different
        // shortcut (e.g. a rebind transaction), unregister the
        // old id so the OS view matches the registry's. The new
        // registration above already replaced the handler in the
        // plugin's internal map; this only removes the obsolete
        // id mapping from our local state.
        if let Some(prev) = guard.registered.remove(&action) {
            guard.id_to_action.remove(&prev.shortcut.id());
        }

        let interned = intern_canonical(canonical);
        guard.id_to_action.insert(shortcut.id(), action);
        guard.registered.insert(
            action,
            RegisteredShortcut {
                shortcut,
                canonical: interned,
            },
        );
        Ok(())
    }

    fn unregister(&self, action: HotkeyAction) {
        let mut guard = self.state.lock();
        let Some(prev) = guard.registered.remove(&action) else {
            return;
        };
        guard.id_to_action.remove(&prev.shortcut.id());
        if let Some(handle) = guard.handle.as_ref() {
            if let Err(err) = handle.global_shortcut().unregister(prev.shortcut) {
                log::debug!(
                    "tauri_global_shortcut_backend: unregister({action:?}) failed: {err:?}"
                );
            }
        }
    }

    fn currently_registered(&self, action: HotkeyAction) -> Option<String> {
        self.state
            .lock()
            .registered
            .get(&action)
            .map(|r| r.canonical.to_string())
    }
}

/// Emit the secondary-launch intent for the matching chord
/// press. Pulled out so the handler closure stays focused on
/// the dispatch. The `HotkeyAction -> SecondaryLaunchIntent`
/// mapping lives in [`pixelgrab_contracts::hotkey`] as a
/// `From` impl so the tray menu, the singleton argv parser,
/// and this handler cannot drift. The channel name is the one
/// the tray menu and the single-instance plugin use, so all
/// three entry-points converge on the same frontend listener.
fn emit_secondary_launch<R: Runtime>(handle: &AppHandle<R>, action: HotkeyAction) {
    let intent: SecondaryLaunchIntent = action.into();
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.emit(SINGLE_INSTANCE_EVENT, &intent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the canonical → plugin Shortcut conversion so the
    /// production backend's parser and the upstream
    /// `global-hotkey` parser never drift. Every binding the
    /// registry hands the backend must round-trip through
    /// `parse_shortcut`.
    #[test]
    fn parse_shortcut_accepts_canonical_grammar() {
        for raw in [
            "CommandOrControl+Shift+S",
            "Alt+F4",
            "Ctrl+Shift+F12",
            "Super+Space",
        ] {
            let shortcut =
                TauriGlobalShortcutBackend::parse_shortcut(raw).expect("canonical parses");
            // The id is derived from `(mods.bits() << 16) | key`,
            // so a stable id for a stable chord is the strongest
            // guarantee we can offer without running the OS hook.
            let again =
                TauriGlobalShortcutBackend::parse_shortcut(raw).expect("canonical re-parses");
            assert_eq!(shortcut.id(), again.id(), "id must be stable for {raw}");
        }
    }

    #[test]
    fn parse_shortcut_rejects_malformed_input() {
        assert!(TauriGlobalShortcutBackend::parse_shortcut("Bogus+S").is_err());
        assert!(TauriGlobalShortcutBackend::parse_shortcut("Ctrl+Shift+").is_err());
    }

    /// Tracer 14 follow-up: the parser must surface a categorical
    /// `PlatformErrorKind::InvalidPayload` rather than the raw
    /// upstream `global-hotkey` Display, which can include user-
    /// supplied paths on some platforms (AGENTS.md §9).
    #[test]
    fn parse_shortcut_error_is_categorically_scrubbed() {
        let err = TauriGlobalShortcutBackend::parse_shortcut("Bogus+S")
            .expect_err("malformed binding must reject");
        assert_eq!(err.kind, PlatformErrorKind::InvalidPayload);
        // The Display string must not contain the offending
        // chord verbatim — only a categorical label + length.
        let msg = format!("{err:?}");
        assert!(
            !msg.contains("Bogus"),
            "raw input must not leak through the IPC message; got {msg:?}"
        );
        assert!(
            msg.contains("invalid_shortcut"),
            "categorical label must be present; got {msg:?}"
        );
    }
}
