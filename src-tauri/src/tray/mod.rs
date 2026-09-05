//! Resident tray setup. Tracer 14 completes the menu with all six
//! promised entries (Capture Region, Capture Full Screen, Shelf
//! History, Pause Global Hotkeys, Settings, Exit) and turns the
//! shortcut hints into live labels that mirror the persisted
//! `HotkeyBindings`.
//!
//! ## State machine
//!
//! The tray owns a [`TrayState`] handle that the binary installs
//! during the Tauri setup hook. The handle holds stable
//! references to each menu item and to the tray icon itself, so
//! status changes (pause toggle, registration error, rebind) can
//! update the labels and tooltip in place without rebuilding the
//! menu.
//!
//! ## Single source of truth
//!
//! Every tray action funnels through the same intent the global
//! shortcut + secondary launch paths use, so the frontend only
//! needs one handler per action. The wiring lives in
//! [`crate::singleton`] (intent parser) and [`crate::ipc`] (the
//! intent handlers).

use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime, Wry,
};

use pixelgrab_contracts::{
    display_binding, HotkeyAction, HotkeyBindings, HotkeyRegistryStatus, SecondaryLaunchIntent,
};

use crate::hotkey::HotkeyRegistry;
use crate::singleton::SINGLE_INSTANCE_EVENT as INTENT_EVENT;

/// Tray menu state. Cheap to clone: every field is pinned behind
/// an `Arc`, so cloning the handle is the same as cloning the
/// installed menu + icon set.
#[derive(Clone)]
pub struct TrayState {
    inner: Arc<TrayInner>,
}

struct TrayInner {
    /// Snapshot of the configured bindings at the moment each
    /// label was rendered. Useful for the diff-based test that
    /// pins "refresh re-renders only on a real change".
    last_bindings: Mutex<HotkeyBindings>,
    /// Strong handle to every menu item the installer created.
    /// Tauri 2 lets a `MenuItem` outlive the parent menu — the
    /// `Arc` here pins it for the process lifetime.
    items: TrayItems,
    /// Strong handle to the tray icon (for `set_tooltip`).
    icon: TrayIcon<Wry>,
}

/// Stable handles to every menu entry the tray installer created.
/// Field order matches `HotkeyAction::ALL` for symmetry with the
/// shortcut-hint code.
struct TrayItems {
    region_capture: MenuItem<Wry>,
    full_screen_capture: MenuItem<Wry>,
    shelf_toggle: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    exit: MenuItem<Wry>,
}

/// Install the resident tray icon and menu, then return the
/// handle the rest of the app can use to drive status updates.
/// Free-function form used by the binary's setup hook. Delegates
/// to [`TrayState::install_with_bindings`].
pub fn install_with_bindings(
    app: &AppHandle<Wry>,
    bindings: &HotkeyBindings,
) -> tauri::Result<TrayState> {
    TrayState::install_with_bindings(app, bindings)
}

impl TrayState {
    /// Install the resident tray icon and menu, then return the
    /// handle the rest of the app can use to drive status updates.
    /// Returns the concrete runtime `Wry` items since the binary
    /// always uses Tauri's default runtime.
    pub fn install_with_bindings(
        app: &AppHandle<Wry>,
        bindings: &HotkeyBindings,
    ) -> tauri::Result<TrayState> {
        let items = build_menu(app)?;
        let menu = Menu::with_items(
            app,
            &[
                &items.region_capture,
                &items.full_screen_capture,
                &items.shelf_toggle,
                &items.pause,
                &items.settings,
                &PredefinedMenuItem::separator(app)?,
                &items.exit,
            ],
        )?;
        let icon = placeholder_icon();
        let tray = TrayIconBuilder::with_id("pixelgrab-tray")
            .icon(icon)
            .tooltip(initial_tooltip(bindings))
            .menu(&menu)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "capture_region" => forward_intent(app, SecondaryLaunchIntent::CaptureRegion),
                "capture_full" => forward_intent(app, SecondaryLaunchIntent::CaptureFullScreen),
                "shelf_history" => forward_intent(app, SecondaryLaunchIntent::ShelfHistory),
                "pause_hotkeys" => handle_pause_hotkey(app),
                "settings" => forward_intent(app, SecondaryLaunchIntent::OpenSettings),
                "exit" => app.exit(0),
                _ => log::debug!("tray menu ignored: {:?}", event.id),
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button,
                    button_state,
                    ..
                } = event
                {
                    if should_capture_from_tray_click(button, button_state) {
                        // Left-click is the pointer equivalent of the global
                        // region shortcut. Route it through the same intent
                        // event so capture lifecycle stays single-sourced.
                        forward_intent(tray.app_handle(), SecondaryLaunchIntent::CaptureRegion);
                    }
                }
            })
            .build(app)?;
        // Render labels for the first time so the user sees the
        // configured shortcuts before the first status update.
        let state = TrayState {
            inner: Arc::new(TrayInner {
                last_bindings: Mutex::new(bindings.clone()),
                items,
                icon: tray,
            }),
        };
        state.refresh_labels(bindings, &initial_status(bindings));
        Ok(state)
    }

    /// Refresh the menu labels + tooltip from a status snapshot.
    /// Called by the IPC layer after every rebind, pause, or
    /// resume so the UI mirrors the latest binding.
    pub fn update_status(&self, bindings: &HotkeyBindings, status: &HotkeyRegistryStatus) {
        self.refresh_labels(bindings, status);
    }

    /// Surface a failed background capture without showing or focusing the
    /// companion window. The next hotkey/tray status refresh restores the
    /// normal tooltip and icon.
    pub fn show_capture_error(&self) {
        let _ = self.inner.icon.set_tooltip(Some(
            "PixelGrab - capture failed; open PixelGrab for details",
        ));
        let _ = self.inner.icon.set_icon(Some(icon_for_capture_error()));
    }

    /// Explicit "hide this tray" call used during shutdown so the
    /// icon disappears before the process exits.
    pub fn shutdown(&self) {
        // Setting the tooltip to empty + hiding the icon gives
        // the OS enough information to remove the entry from the
        // notification area.
        let _ = self.inner.icon.set_tooltip(Some(""));
        let _ = self.inner.icon.set_visible(false);
    }

    fn refresh_labels(&self, bindings: &HotkeyBindings, status: &HotkeyRegistryStatus) {
        let label = |action: HotkeyAction, kind: &str| {
            let binding = bindings.get(action).map(display_binding);
            match binding {
                Some(hint) => format!("{kind}\u{2003}\u{2014}\u{2003}{hint}"),
                None => kind.to_string(),
            }
        };
        set_item_text(
            &self.inner.items.region_capture,
            &label(HotkeyAction::RegionCapture, "Capture Region"),
        );
        set_item_text(
            &self.inner.items.full_screen_capture,
            &label(HotkeyAction::FullScreenCapture, "Capture Full Screen"),
        );
        set_item_text(
            &self.inner.items.shelf_toggle,
            &label(HotkeyAction::ShelfToggle, "Shelf History"),
        );
        set_item_text(
            &self.inner.items.pause,
            if status.paused {
                "Resume Global Hotkeys"
            } else {
                "Pause Global Hotkeys"
            },
        );
        set_item_text(&self.inner.items.settings, "Settings");
        set_item_text(&self.inner.items.exit, "Exit");
        let tooltip = compose_tooltip(bindings, status);
        let _ = self.inner.icon.set_tooltip(Some(&tooltip));
        // Swap the icon to mirror the current state — spec asks
        // for "icon state" to reflect paused / error alongside the
        // tooltip text. set_icon replaces the OS-level resource so
        // the swap is visible immediately.
        let _ = self.inner.icon.set_icon(Some(icon_for_status(status)));
        // Cache for the next refresh; this keeps the diff
        // observable to tests.
        *self.inner.last_bindings.lock() = bindings.clone();
    }
}

/// Return whether a native tray click should start a region capture. Keeping
/// this predicate separate makes the pointer contract testable without a
/// live Windows notification-area icon.
pub(crate) fn should_capture_from_tray_click(
    button: MouseButton,
    button_state: MouseButtonState,
) -> bool {
    matches!(
        (button, button_state),
        (MouseButton::Left, MouseButtonState::Down)
    )
}

fn set_item_text<R: Runtime>(item: &MenuItem<R>, text: &str) {
    if let Err(err) = item.set_text(text) {
        // The menu item is held by an Arc — set_text cannot
        // return Err under normal circumstances; logging the
        // error is the most we can do without crashing.
        log::debug!("tray menu: set_text failed for {text:?}: {err:?}");
    }
}

/// Forward a tray-initiated intent to the frontend listener.
/// The frontend already handles `pixelgrab://request-capture`; for
/// the remaining actions we emit a typed payload (on
/// `INTENT_EVENT`, re-exported from `crate::singleton` as
/// `SINGLE_INSTANCE_EVENT`) so the listener can route them to the
/// matching IPC handler.
fn forward_intent<R: Runtime>(app: &AppHandle<R>, intent: SecondaryLaunchIntent) {
    if matches!(intent, SecondaryLaunchIntent::Default) {
        // Default means "just focus me" — we still want to focus
        // the window so the icon click feels responsive.
        focus_main_window(app);
        return;
    }
    if matches!(intent, SecondaryLaunchIntent::ShelfHistory) {
        if let Some(state) = app.try_state::<crate::PixelGrabApp>() {
            if let Err(_err) = crate::ipc::show_shelf_queue_native(&state, app) {
                log::warn!("tray shelf history presentation failed");
            }
        } else {
            log::warn!("tray shelf history state unavailable");
        }
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        // Capture intents must leave PixelGrab's companion hidden. Showing it
        // before the backend freezes the desktop captures our own window.
        if !matches!(
            &intent,
            SecondaryLaunchIntent::CaptureRegion
                | SecondaryLaunchIntent::CaptureFullScreen
                | SecondaryLaunchIntent::ShelfHistory
        ) {
            focus_main_window(app);
        }
        let _ = window.emit(INTENT_EVENT, &intent);
    }
}

/// Internal: open the settings panel from the tray. The frontend
/// focuses the panel via the `pixelgrab://secondary-launch` event;
/// this helper exists so a future "first-run wizard" path can wire
/// through the same hook without re-typing the channel name.
fn handle_pause_hotkey<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(PAUSE_TOGGLE_EVENT, ());
    }
}

/// Convenience: bring the main window to the foreground. A tray
/// click is always followed by a focus so the user sees their
/// action landing immediately.
fn focus_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Build a fresh menu (items returned so the caller can stash
/// them in the `TrayState`). Each `with_id` call panics on a
/// duplicate id, so the ids below must remain unique.
fn build_menu(app: &AppHandle<Wry>) -> tauri::Result<TrayItems> {
    let region_capture =
        MenuItem::with_id(app, "capture_region", "Capture Region", true, None::<&str>)?;
    let full_screen_capture = MenuItem::with_id(
        app,
        "capture_full",
        "Capture Full Screen",
        true,
        None::<&str>,
    )?;
    let shelf_toggle =
        MenuItem::with_id(app, "shelf_history", "Shelf History", true, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        "pause_hotkeys",
        "Pause Global Hotkeys",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
    Ok(TrayItems {
        region_capture,
        full_screen_capture,
        shelf_toggle,
        pause,
        settings,
        exit,
    })
}

/// Initial tooltip used during the install call. The real tooltip
/// arrives with the first `update_status`; this string just keeps
/// the tray readable for the brief moment between install and
/// first refresh.
fn initial_tooltip(bindings: &HotkeyBindings) -> String {
    compose_tooltip(bindings, &initial_status(bindings))
}

fn initial_status(bindings: &HotkeyBindings) -> HotkeyRegistryStatus {
    HotkeyRegistryStatus {
        active: !bindings.paused && HotkeyAction::ALL.iter().any(|a| bindings.get(*a).is_some()),
        paused: bindings.paused,
        last_error: None,
        conflicting_action: None,
    }
}

fn compose_tooltip(bindings: &HotkeyBindings, status: &HotkeyRegistryStatus) -> String {
    if let Some(action) = status.conflicting_action {
        let label = bindings
            .get(action)
            .map(display_binding)
            .unwrap_or_else(|| "<unset>".to_string());
        return format!(
            "PixelGrab \u{2014} registration failed for {} ({label}). Open Settings to recover.",
            action.label()
        );
    }
    if status.paused {
        return "PixelGrab \u{2014} global hotkeys paused".to_string();
    }
    "PixelGrab \u{2014} capture ready".to_string()
}

fn icon_for_capture_error() -> Image<'static> {
    icons::conflict()
}

/// Stable name for the tray-driven pause toggle event. The
/// frontend forwards this to `set_paused` via the IPC.
pub const PAUSE_TOGGLE_EVENT: &str = "pixelgrab://pause-hotkeys-toggled";

/// Apply a registry status to the tray. Convenience wrapper so
/// the binary does not need to know about `TrayState`'s
/// internals.
pub fn refresh_tray(tray: &TrayState, registry: &HotkeyRegistry) {
    let bindings = registry.current_bindings();
    let status = registry.status();
    tray.update_status(&bindings, &status);
}

/// Validate that a menu id is one of the canonical six. Avoids a
/// typo in the IPC layer silently dropping tray clicks.
#[allow(dead_code)]
pub(crate) fn known_menu_id(id: &str) -> bool {
    matches!(
        id,
        "capture_region" | "capture_full" | "shelf_history" | "pause_hotkeys" | "settings" | "exit"
    )
}

/// Composed icon palette. The 32x32 RGBA buffers live as static
/// slices so swapping the icon does not allocate. Future work can
/// swap these for real PNG assets via the tauri codegen path.
mod icons {
    use super::Image;

    const SIZE: u32 = 32;
    const LEN: usize = (SIZE * SIZE * 4) as usize;

    /// Pre-rendered 32x32 RGBA buffer for the icon variants.
    /// Each variant is a single coloured square — the production
    /// UX can layer a gradient on top without changing the wiring.
    fn solid(r: u8, g: u8, b: u8) -> Image<'static> {
        let mut buf = vec![0u8; LEN];
        for y in 0..SIZE {
            for x in 0..SIZE {
                let i = ((y * SIZE + x) * 4) as usize;
                buf[i] = r;
                buf[i + 1] = g;
                buf[i + 2] = b;
                buf[i + 3] = 0xFF;
            }
        }
        Image::new_owned(buf, SIZE, SIZE)
    }

    /// Idle: blue (capture ready).
    pub fn idle() -> Image<'static> {
        solid(0x4E, 0xA1, 0xFF)
    }

    /// Paused: amber (no hooks active).
    pub fn paused() -> Image<'static> {
        solid(0xFF, 0xB3, 0x4D)
    }

    /// Conflict: red (one or more bindings were rejected by the
    /// backend).
    pub fn conflict() -> Image<'static> {
        solid(0xE5, 0x4B, 0x4B)
    }
}

/// Pick the icon that matches the current registry status and
/// push it onto the tray. Called from `refresh_labels` after the
/// tooltip is composed so the icon always matches the text.
fn icon_for_status(status: &HotkeyRegistryStatus) -> Image<'static> {
    if status.conflicting_action.is_some() {
        return icons::conflict();
    }
    if status.paused {
        return icons::paused();
    }
    icons::idle()
}

/// Minimal in-memory RGBA used as the tray icon at install time.
/// `refresh_labels` swaps in a status-coloured icon as soon as the
/// first status refresh lands.
fn placeholder_icon() -> Image<'static> {
    icons::idle()
}

/// Validate that a transition between two statuses is well-formed.
/// Pulled out so the integration tests can pin the rule.
#[allow(dead_code)]
pub(crate) fn compose_shortcut_hints(bindings: &HotkeyBindings) -> Vec<String> {
    HotkeyAction::ALL
        .iter()
        .map(|a| {
            let label = a.label();
            match bindings.get(*a) {
                Some(canonical) => format!("{label}: {}", display_binding(canonical)),
                None => format!("{label}: (unbound)"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_shortcut_hints_lists_three_labels() {
        let hints = compose_shortcut_hints(&HotkeyBindings::defaults());
        assert_eq!(hints.len(), 3);
        assert!(hints.iter().any(|l| l.contains("Capture Region")));
        assert!(hints.iter().any(|l| l.contains("Capture Full Screen")));
        assert!(hints.iter().any(|l| l.contains("Toggle Shelf")));
    }

    #[test]
    fn initial_status_mirrors_paused_flag() {
        let bindings = HotkeyBindings::defaults();
        assert!(initial_status(&bindings).active);
        let mut paused = HotkeyBindings::defaults();
        paused.paused = true;
        assert!(!initial_status(&paused).active);
    }

    #[test]
    fn compose_tooltip_changes_for_paused_and_error() {
        let bindings = HotkeyBindings::defaults();
        let happy = compose_tooltip(&bindings, &initial_status(&bindings));
        assert!(happy.contains("capture ready"));
        let paused_status = HotkeyRegistryStatus {
            active: false,
            paused: true,
            last_error: None,
            conflicting_action: None,
        };
        let paused = compose_tooltip(&bindings, &paused_status);
        assert!(paused.contains("paused"));
        let error_status = HotkeyRegistryStatus {
            active: false,
            paused: false,
            last_error: Some("held".into()),
            conflicting_action: Some(HotkeyAction::ShelfToggle),
        };
        let with_error = compose_tooltip(&bindings, &error_status);
        assert!(with_error.contains("registration failed"));
    }

    #[test]
    fn known_menu_id_accepts_every_six_label() {
        for id in [
            "capture_region",
            "capture_full",
            "shelf_history",
            "pause_hotkeys",
            "settings",
            "exit",
        ] {
            assert!(known_menu_id(id), "{id} must be accepted");
        }
        assert!(!known_menu_id("nope"));
    }

    #[test]
    fn left_tray_press_starts_region_capture_only() {
        assert!(should_capture_from_tray_click(
            MouseButton::Left,
            MouseButtonState::Down
        ));
        assert!(!should_capture_from_tray_click(
            MouseButton::Right,
            MouseButtonState::Down
        ));
        assert!(!should_capture_from_tray_click(
            MouseButton::Left,
            MouseButtonState::Up
        ));
    }
}
