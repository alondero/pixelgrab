//! Hotkey registry: ties user-configured shortcut strings to the
//! Tauri global-shortcut plugin.
//!
//! Tracer 14 lifts the resident-tray shortcut configuration out of
//! its hard-coded shell. The registry is the runtime state machine
//! behind the [`crate::ipc::commands`] hotkey IPCs and the tray
//! "Pause Global Hotkeys" entry. The state machine has only three
//! knobs:
//!
//! - **Apply** — install the currently configured bindings on the
//!   OS. The first apply at process start loads the persisted
//!   bindings, registers each binding with the backend, and records
//!   the result in [`HotkeyRegistryStatus`].
//! - **Rebind** — update a single binding atomically. The
//!   transaction tries to register the proposed binding with the
//!   backend before mutating in-memory state; a backend rejection
//!   is reported back as a [`HotkeyConflict`] and the previous
//!   working binding stays active.
//! - **Pause / Resume** — flip a runtime flag. Pausing unregisters
//!   all hooks so the OS stops dispatching them; resume
//!   re-registers without dropping the user's configured strings,
//!   so a paused → resume cycle never asks for fresh shortcuts.
//!
//! The backend trait is the seam for CI: the in-memory fake never
//! touches the OS, while the Tauri adapter wraps
//! `tauri-plugin-global-shortcut`'s state. The tests in this
//! module use the fake exclusively.

pub mod store;
pub mod tauri_backend;

pub use tauri_backend::TauriGlobalShortcutBackend;

use parking_lot::Mutex;
use pixelgrab_contracts::{
    display_binding, parse_binding, validate_for_storage, HotkeyAction, HotkeyBindings,
    HotkeyRegistryStatus, PlatformError, PlatformErrorKind,
};
use std::collections::HashSet;
use std::sync::Arc;

/// Outcome of a rebind that the backend rejected. Surfaced to the
/// frontend so the settings UI can point at the offending field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyConflict {
    /// The action whose proposed binding the OS rejected.
    pub action: HotkeyAction,
    /// The proposed (canonicalised) binding.
    pub binding: String,
    /// A short, user-facing reason. Categorical kinds so the
    /// message stays free of OS-specific text that could leak
    /// keyboard paths through telemetry.
    pub reason: &'static str,
}

impl HotkeyConflict {
    fn binding_held_by_other_process(action: HotkeyAction, binding: String) -> Self {
        Self {
            action,
            binding,
            reason: "binding_held_by_other_process",
        }
    }

    fn binding_held_by_other_action(action: HotkeyAction, binding: String) -> Self {
        Self {
            action,
            binding,
            reason: "binding_already_used",
        }
    }

    fn registration_failed(action: HotkeyAction, binding: String) -> Self {
        Self {
            action,
            binding,
            reason: "registration_failed",
        }
    }
}

/// Hotkey registration backend. Production wraps
/// `tauri-plugin-global-shortcut`; tests use the in-memory fake so
/// no OS interaction occurs.
pub trait GlobalShortcutBackend: std::fmt::Debug + Send + Sync {
    /// Register the given binding for the action. Returns `Err`
    /// when the OS rejects the binding (e.g. another process holds
    /// it).
    fn register(&self, action: HotkeyAction, canonical: &str) -> Result<(), PlatformError>;
    /// Unregister the action's current binding. A no-op when the
    /// action was never registered.
    fn unregister(&self, action: HotkeyAction);
    /// Return the action's currently registered binding, if any.
    fn currently_registered(&self, action: HotkeyAction) -> Option<String>;
}

/// In-memory fake of the OS backend. Used by every test that does
/// not need a real global hotkey registration; the only thing it
/// cannot reproduce is the OS rejecting a binding due to an
/// external process — tests cover that case by seeding the
/// `externally_held` set.
#[derive(Debug, Default)]
pub struct InMemoryBackend {
    inner: Mutex<InMemoryBackendState>,
}

#[derive(Debug, Default)]
struct InMemoryBackendState {
    registered: std::collections::HashMap<HotkeyAction, String>,
    rejected_extra: HashSet<String>,
}

impl InMemoryBackend {
    /// Build a fresh in-memory backend.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Configure the backend to reject every binding in
    /// `bindings`. Used to simulate "another process holds this
    /// accelerator" without involving the OS.
    pub fn reject(&self, bindings: impl IntoIterator<Item = String>) {
        let mut guard = self.inner.lock();
        guard.rejected_extra.extend(bindings);
    }

    /// Clear the rejection list, returning the backend to "always
    /// accept new bindings".
    pub fn clear_rejections(&self) {
        self.inner.lock().rejected_extra.clear();
    }

    /// Snapshot of every currently registered (action, binding)
    /// pair. Used by the integration tests for direct assertions.
    pub fn snapshot(&self) -> Vec<(HotkeyAction, String)> {
        let guard = self.inner.lock();
        let mut entries: Vec<_> = guard
            .registered
            .iter()
            .map(|(a, b)| (*a, b.clone()))
            .collect();
        entries.sort_by_key(|(a, _)| *a as u8);
        entries
    }
}

impl GlobalShortcutBackend for InMemoryBackend {
    fn register(&self, action: HotkeyAction, canonical: &str) -> Result<(), PlatformError> {
        let mut guard = self.inner.lock();
        if guard.rejected_extra.contains(canonical) {
            return Err(PlatformError::new(
                PlatformErrorKind::Internal,
                format!("backend rejected binding: {canonical}"),
            ));
        }
        if let Some(existing) = guard.registered.get(&action) {
            if existing == canonical {
                // Idempotent re-register — pretend it succeeded.
                return Ok(());
            }
            // Different binding for the same action: replace.
            guard.registered.insert(action, canonical.to_string());
            return Ok(());
        }
        // New action: check whether another action already holds
        // the same binding (PixelGrab cannot bind two actions to
        // the same accelerator).
        if guard.registered.values().any(|b| b.as_str() == canonical) {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("binding {canonical} already in use"),
            ));
        }
        guard.registered.insert(action, canonical.to_string());
        Ok(())
    }

    fn unregister(&self, action: HotkeyAction) {
        let mut guard = self.inner.lock();
        guard.registered.remove(&action);
    }

    fn currently_registered(&self, action: HotkeyAction) -> Option<String> {
        self.inner.lock().registered.get(&action).cloned()
    }
}

/// Outcome of a single registry call. Public so tray/IPC callers
/// can map it onto the wire shape without leaking internal types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryOutcome {
    /// State changed (or was already at the requested value).
    NoChange,
    /// State changed.
    Changed,
    /// The backend rejected the proposed binding. The previous
    /// working binding remains registered.
    Conflict(HotkeyConflict),
}

impl RegistryOutcome {
    /// `true` when the registry actually mutated its state.
    pub fn changed(&self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// Runtime registry. Cheap to clone: every field is behind an
/// `Arc` or a `Mutex`. The Tauri setup hook installs one as a
/// managed state value (see [`crate::PixelGrabApp::hotkeys`]).
#[derive(Debug, Clone)]
pub struct HotkeyRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Debug)]
struct RegistryInner {
    /// Persisted + in-memory bindings document. Owned here so the
    /// tray / IPC can read the latest values without crossing an
    /// IPC boundary; the persistence layer mirrors this on every
    /// flush_blocking call.
    bindings: Mutex<HotkeyBindings>,
    /// Last computed status payload. Updated on every apply /
    /// rebind / pause.
    status: Mutex<HotkeyRegistryStatus>,
    /// OS-level backend. Production wraps
    /// `tauri-plugin-global-shortcut`; tests use `InMemoryBackend`.
    backend: Arc<dyn GlobalShortcutBackend>,
}

impl HotkeyRegistry {
    /// Build a new registry backed by the supplied backend. The
    /// initial bindings default to [`HotkeyBindings::defaults`].
    pub fn new(backend: Arc<dyn GlobalShortcutBackend>) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                bindings: Mutex::new(HotkeyBindings::defaults()),
                status: Mutex::new(HotkeyRegistryStatus {
                    active: false,
                    paused: false,
                    last_error: None,
                    conflicting_action: None,
                }),
                backend,
            }),
        }
    }

    /// Build a registry pre-loaded with the supplied bindings. The
    /// configured bindings are *not* registered with the backend
    /// until [`HotkeyRegistry::apply`] is called; this lets the
    /// Tauri setup hook install the tray menu before the global
    /// shortcut plugin has been wired.
    pub fn with_bindings(
        backend: Arc<dyn GlobalShortcutBackend>,
        bindings: HotkeyBindings,
    ) -> Self {
        let mut status = HotkeyRegistryStatus {
            active: false,
            paused: bindings.paused,
            last_error: None,
            conflicting_action: None,
        };
        if bindings.paused {
            // Paused => not active. Other fields stay default.
            status.active = false;
        }
        Self {
            inner: Arc::new(RegistryInner {
                bindings: Mutex::new(bindings),
                status: Mutex::new(status),
                backend,
            }),
        }
    }

    /// Snapshot of the registry's status payload. Read-only.
    pub fn status(&self) -> HotkeyRegistryStatus {
        self.inner.status.lock().clone()
    }

    /// Read-only access to the configured bindings. The
    /// preferences layer calls this to dump a fresh copy before
    /// persisting.
    pub fn current_bindings(&self) -> HotkeyBindings {
        self.inner.bindings.lock().clone()
    }

    /// Replace the configured bindings in memory. Does NOT register
    /// or unregister with the backend; the caller decides when to
    /// apply. Used by the startup hot-path so a malformed
    /// persisted file does not crash the IPC handler that
    /// triggered the load.
    pub fn set_bindings(&self, bindings: HotkeyBindings) {
        let mut guard = self.inner.bindings.lock();
        let paused = bindings.paused;
        *guard = bindings;
        // Mirror into status. Active stays false until apply.
        let mut status = self.inner.status.lock();
        status.paused = paused;
    }

    /// Apply the current bindings to the backend. Paused
    /// registries unregister everything instead. Returns the
    /// `RegistryOutcome::Conflict` for the first action that
    /// failed; on success every binding is registered and the
    /// status reads `active = true, paused = false`.
    pub fn apply(&self) -> RegistryOutcome {
        let bindings = self.inner.bindings.lock().clone();
        let paused = bindings.paused;
        let mut status = self.inner.status.lock();

        // Pause path: unregister every action and report inactive.
        if paused {
            for action in HotkeyAction::ALL {
                self.inner.backend.unregister(*action);
            }
            status.active = false;
            status.paused = true;
            status.last_error = None;
            status.conflicting_action = None;
            return RegistryOutcome::Changed;
        }

        // Active path: register every configured binding,
        // tearing back any partially-registered bindings on the
        // first failure.
        let mut registered_now: Vec<HotkeyAction> = Vec::new();
        for action in HotkeyAction::ALL {
            let Some(raw) = bindings.get(*action) else {
                // Unbound action: ensure it is not lingering from a
                // previous run.
                self.inner.backend.unregister(*action);
                continue;
            };
            match self.inner.backend.register(*action, raw) {
                Ok(()) => {
                    log::info!("hotkey: registered {action:?} -> {raw}");
                    registered_now.push(*action);
                }
                Err(err) => {
                    log::warn!(
                        "hotkey: registration FAILED for {action:?} -> {raw}: kind={:?} msg={}",
                        err.kind,
                        err
                    );
                    // Rollback the bindings we managed to register
                    // so a half-applied state cannot keep OS handles
                    // out of sync with the configured bindings.
                    for done in &registered_now {
                        self.inner.backend.unregister(*done);
                    }
                    status.active = false;
                    status.paused = false;
                    let err_str = err.to_string();
                    status.last_error = Some(err_str);
                    status.conflicting_action = Some(*action);
                    return RegistryOutcome::Conflict(HotkeyConflict::registration_failed(
                        *action,
                        raw.to_string(),
                    ));
                }
            }
        }
        log::info!(
            "hotkey: all {} bindings registered; active={} paused={}",
            registered_now.len(),
            status.active,
            status.paused
        );
        status.active = true;
        status.paused = false;
        status.last_error = None;
        status.conflicting_action = None;
        RegistryOutcome::Changed
    }

    /// Update the bindings wholesale and apply them. IPC-layer
    /// convenience for the settings UI's "Save" button: the
    /// in-memory state and the OS handle set are updated
    /// together; on any failure the previous bindings + OS
    /// handles are restored so the user never observes a
    /// half-applied rebind.
    pub fn apply_replacements(&self, new_bindings: &HotkeyBindings) -> Result<(), HotkeyConflict> {
        // Capture the previous in-memory copy so we can roll back
        // when the backend refuses at least one of the new
        // bindings.
        let previous_bindings = self.inner.bindings.lock().clone();
        // Canonicalise each slot before applying so the in-memory
        // state mirrors what the OS sees (and what the disk will
        // see after the next persistence write).
        let canonical_region = new_bindings
            .region_capture
            .as_deref()
            .and_then(parse_binding);
        let canonical_full = new_bindings
            .full_screen_capture
            .as_deref()
            .and_then(parse_binding);
        let canonical_shelf = new_bindings.shelf_toggle.as_deref().and_then(parse_binding);
        // Commit the canonical bindings + paused flag to memory.
        {
            let mut bindings = self.inner.bindings.lock();
            bindings.schema_version = new_bindings.schema_version;
            bindings.region_capture = canonical_region;
            bindings.full_screen_capture = canonical_full;
            bindings.shelf_toggle = canonical_shelf;
            bindings.paused = new_bindings.paused;
        }
        let outcome = self.apply();
        match outcome {
            RegistryOutcome::Conflict(conflict) => {
                // Roll back to the previous in-memory state.
                *self.inner.bindings.lock() = previous_bindings.clone();
                // Ensure the OS is back in sync with the rolled-
                // back in-memory state.
                let _ = self.apply();
                Err(conflict)
            }
            _ => Ok(()),
        }
    }

    /// Update one binding transactionally. The candidate binding
    /// is registered with the backend first; only on success does
    /// the in-memory `bindings` field update, and only on success
    /// is the previous binding unregistered. A backend rejection
    /// leaves the previous binding live.
    ///
    /// Passing `raw = None` or an empty string unbinds the action
    /// (unregisters the OS handle, leaves the slot None).
    pub fn rebind(
        &self,
        action: HotkeyAction,
        raw: Option<&str>,
    ) -> Result<RegistryOutcome, PlatformError> {
        let canonical = match raw {
            None | Some("") => None,
            Some(value) => {
                let parsed = parse_binding(value).ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorKind::InvalidPayload,
                        format!(
                            "{}: {value:?} is not a recognised accelerator",
                            action.as_id()
                        ),
                    )
                })?;
                Some(parsed)
            }
        };
        if let Some(canonical) = canonical.as_deref() {
            // Reject intra-PixelGrab duplicates *before* hitting the
            // backend; this guard is duplicated by the backend but
            // catching it here gives the caller a precise reason.
            for other in HotkeyAction::ALL {
                if *other == action {
                    continue;
                }
                let registered = self.inner.backend.currently_registered(*other);
                if registered.as_deref() == Some(canonical) {
                    return Ok(RegistryOutcome::Conflict(
                        HotkeyConflict::binding_held_by_other_action(action, canonical.to_string()),
                    ));
                }
            }
        }

        // Try-before-commit. The backend is asked to register the
        // candidate before we ever drop the current binding.
        if let Some(canonical) = canonical.as_deref() {
            if let Err(_err) = self.inner.backend.register(action, canonical) {
                return Ok(RegistryOutcome::Conflict(
                    HotkeyConflict::binding_held_by_other_process(action, canonical.to_string()),
                ));
            }
        }

        // Commit point: update in-memory bindings, then drop the
        // prior binding ONLY when the in-memory change actually
        // took effect. Dropping the prior binding before knowing
        // whether the new binding is going to stick would risk
        // leaving the OS without any registration.
        let changed = {
            let mut bindings = self.inner.bindings.lock();
            match canonical.as_ref() {
                Some(c) => bindings.set(action, Some(c.clone())),
                None => bindings.set(action, None),
            }
        };
        if !changed {
            // No-op rebind: keep the previous registration live.
            // The try-before-commit above may have already
            // registered the candidate; if the previous binding
            // was different, re-register the previous one.
            if let Some(current) = self.current_registration(action) {
                if canonical.as_deref() != Some(current.as_str()) {
                    let _ = self.inner.backend.register(action, &current);
                }
            }
            let mut status = self.inner.status.lock();
            status.last_error = None;
            status.conflicting_action = None;
            return Ok(RegistryOutcome::NoChange);
        }
        // The new binding is live (registered by the
        // try-before-commit step). Drop the prior binding only if
        // it was different from the new one; the backend is
        // idempotent so this is safe.
        let prior = self.current_registration(action);
        if prior.as_deref() != canonical.as_deref() {
            self.inner.backend.unregister(action);
            if let Some(canonical) = canonical.as_deref() {
                let _ = self.inner.backend.register(action, canonical);
            }
        }

        // Refresh status to mirror the new state.
        let mut status = self.inner.status.lock();
        let bindings = self.inner.bindings.lock();
        let paused = bindings.paused;
        let any_active = HotkeyAction::ALL.iter().any(|a| bindings.get(*a).is_some());
        status.active = any_active && !paused;
        status.paused = paused;
        status.last_error = None;
        status.conflicting_action = None;
        Ok(RegistryOutcome::Changed)
    }

    /// Best-effort: read the backend's currently-registered
    /// string for an action.
    fn current_registration(&self, action: HotkeyAction) -> Option<String> {
        self.inner.backend.currently_registered(action)
    }

    /// Toggle paused state. When pausing, every registered action
    /// is unregistered; the configured strings are retained so a
    /// resume repopulates the OS handles without user input.
    /// Returns the new paused value so the tray menu can flip its
    /// label.
    pub fn set_paused(&self, paused: bool) -> bool {
        // Compute the set of changes under the bindings lock first
        // so we never hold that lock while touching `status` or the
        // backend — parking_lot::Mutex is non-reentrant, so a
        // nested acquisition here would deadlock (see the
        // `set_paused_resume_registers_all_current_bindings` test).
        let changed = {
            let mut bindings = self.inner.bindings.lock();
            let changed = bindings.set_paused(paused);
            if !changed {
                return false;
            }
            changed
        };
        let _ = changed;

        if paused {
            // Unregister every action; the strings stay in `bindings`.
            for action in HotkeyAction::ALL {
                self.inner.backend.unregister(*action);
            }
        } else {
            // Resume: only the configured bindings are registered.
            // Failures here are surfaced via the status payload
            // rather than aborting the toggle.
            // Snapshot the configured bindings under the lock so we
            // can release it before talking to the backend.
            let snapshot: Vec<(HotkeyAction, String)> = {
                let bindings = self.inner.bindings.lock();
                HotkeyAction::ALL
                    .iter()
                    .filter_map(|a| bindings.get(*a).map(|s| (*a, s.to_string())))
                    .collect()
            };
            for (action, raw) in &snapshot {
                if let Err(err) = self.inner.backend.register(*action, raw) {
                    let _ = err;
                    // Roll back the partial register so a
                    // half-resumed state cannot leak OS handles,
                    // and flip the in-memory paused flag back to
                    // true so the next call does the right thing.
                    {
                        let mut bindings = self.inner.bindings.lock();
                        bindings.set_paused(true);
                    }
                    for other in HotkeyAction::ALL {
                        self.inner.backend.unregister(*other);
                    }
                    let mut status = self.inner.status.lock();
                    status.last_error = Some(err.to_string());
                    status.conflicting_action = Some(*action);
                    status.active = false;
                    status.paused = true;
                    return false;
                }
            }
        }
        // Compute active outside any lock held elsewhere.
        let any_active = {
            let bindings = self.inner.bindings.lock();
            HotkeyAction::ALL.iter().any(|a| bindings.get(*a).is_some())
        };
        let mut status = self.inner.status.lock();
        status.paused = paused;
        status.active = !paused && any_active;
        status.last_error = None;
        status.conflicting_action = None;
        true
    }

    /// Uninstall every registration. Used at process shutdown so
    /// the OS handles do not outlive the resident process.
    pub fn shutdown(&self) {
        for action in HotkeyAction::ALL {
            self.inner.backend.unregister(*action);
        }
        let mut status = self.inner.status.lock();
        status.active = false;
    }

    /// Validate a proposed binding string for an action. Used by
    /// the IPC layer to short-circuit the user input before
    /// the registry attempts the actual rebind.
    pub fn validate(action: HotkeyAction, raw: &str) -> Result<String, PlatformError> {
        validate_for_storage(action, raw)
    }
}

/// Render the configured binding for an action into the display
/// form expected by the tray menu (and the frontend settings UI).
/// Returns `"unbound"` when no binding is configured.
pub fn format_hint(bindings: &HotkeyBindings, action: HotkeyAction) -> String {
    match bindings.get(action) {
        Some(canonical) => display_binding(canonical),
        None => "unbound".to_string(),
    }
}

/// Build the wire DTO from the current status. The IPC layer uses
/// this to keep the wire shape in one place.
pub fn status_to_dto(
    status: &HotkeyRegistryStatus,
) -> pixelgrab_contracts::HotkeyRegistryStatusDto {
    pixelgrab_contracts::HotkeyRegistryStatusDto {
        active: status.active,
        paused: status.paused,
        last_error: status.last_error.clone(),
        conflicting_action: status.conflicting_action.map(|a| a.as_id().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> (HotkeyRegistry, Arc<InMemoryBackend>) {
        let backend = InMemoryBackend::new();
        let registry = HotkeyRegistry::new(backend.clone());
        (registry, backend)
    }

    #[test]
    fn defaults_register_on_first_apply() {
        let (reg, backend) = registry();
        let outcome = reg.apply();
        assert_eq!(outcome, RegistryOutcome::Changed);
        let snap = backend.snapshot();
        assert_eq!(snap.len(), HotkeyAction::ALL.len());
        assert!(reg.status().active);
        assert!(!reg.status().paused);
    }

    #[test]
    fn apply_when_paused_unregisters_but_keeps_strings() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        // Now flip paused on, applying again should clear the OS handles.
        assert!(reg.set_paused(true));
        let snap = backend.snapshot();
        assert!(snap.is_empty(), "paused registry must not hold any handles");
        let bindings = reg.current_bindings();
        assert!(bindings.paused);
        assert!(
            bindings.region_capture.is_some(),
            "configured strings are retained"
        );
    }

    #[test]
    fn rebind_transactions_roll_back_on_failure() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        // Externally-held binding causes the rebind to fail.
        backend.reject(["CommandOrControl+Alt+R".to_string()]);
        let outcome = reg
            .rebind(HotkeyAction::RegionCapture, Some("Ctrl+Alt+R"))
            .unwrap();
        match outcome {
            RegistryOutcome::Conflict(c) => {
                assert_eq!(c.action, HotkeyAction::RegionCapture);
                assert_eq!(c.binding, "CommandOrControl+Alt+R");
                assert_eq!(c.reason, "binding_held_by_other_process");
            }
            other => panic!("expected conflict, got {other:?}"),
        }
        // Previous binding is still live.
        let registered = backend.currently_registered(HotkeyAction::RegionCapture);
        assert!(
            registered.is_some(),
            "previous binding must remain registered"
        );
    }

    #[test]
    fn rebind_rolls_back_on_intra_duplicate() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        let outcome = reg
            .rebind(
                HotkeyAction::RegionCapture,
                Some("Ctrl+Shift+F"), // held by FullScreenCapture
            )
            .unwrap();
        match outcome {
            RegistryOutcome::Conflict(c) => {
                assert_eq!(c.reason, "binding_already_used");
            }
            other => panic!("expected intra-duplicate conflict, got {other:?}"),
        }
        // Both old bindings remain registered.
        assert_eq!(backend.snapshot().len(), HotkeyAction::ALL.len());
    }

    #[test]
    fn rebind_unbind_clears_registration() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        let outcome = reg.rebind(HotkeyAction::ShelfToggle, None).unwrap();
        assert_eq!(outcome, RegistryOutcome::Changed);
        assert!(backend
            .currently_registered(HotkeyAction::ShelfToggle)
            .is_none());
    }

    #[test]
    fn rebind_idempotent_returns_no_change() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        let original = backend
            .currently_registered(HotkeyAction::RegionCapture)
            .expect("default registered");
        let outcome = reg
            .rebind(HotkeyAction::RegionCapture, Some(&original))
            .unwrap();
        assert_eq!(outcome, RegistryOutcome::NoChange);
    }

    #[test]
    fn rebind_with_malformed_string_returns_err() {
        let (reg, _backend) = registry();
        let result = reg.rebind(HotkeyAction::RegionCapture, Some("nonsense"));
        assert!(
            result.is_err(),
            "malformed string is rejected before OS contact"
        );
    }

    #[test]
    fn set_paused_toggle_idempotent() {
        let (reg, _backend) = registry();
        assert!(reg.set_paused(true));
        assert!(!reg.set_paused(true), "second pause is a no-op");
    }

    #[test]
    fn set_paused_resume_registers_all_current_bindings() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        assert!(reg.set_paused(true));
        assert!(reg.set_paused(false));
        assert_eq!(backend.snapshot().len(), HotkeyAction::ALL.len());
        assert!(reg.status().active);
        assert!(!reg.status().paused);
    }

    #[test]
    fn set_paused_resume_rolls_back_on_register_failure() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        reg.set_paused(true);
        // Refuse one of the configured bindings; the resume must
        // not leave the OS with a partial set.
        backend.reject(["CommandOrControl+Shift+L".to_string()]);
        assert!(!reg.set_paused(false));
        let snap = backend.snapshot();
        assert!(snap.is_empty(), "failed resume must unwind every OS handle");
        assert!(reg.current_bindings().paused, "paused flag rolls back");
    }

    #[test]
    fn shutdown_clears_every_os_handle() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        reg.shutdown();
        assert!(backend.snapshot().is_empty());
        assert!(!reg.status().active);
    }

    #[test]
    fn status_reflects_latest_apply_outcome() {
        let (reg, backend) = registry();
        let _ = reg.apply();
        backend.reject(["CommandOrControl+Shift+S".to_string()]);
        let outcome = reg.apply();
        assert!(matches!(outcome, RegistryOutcome::Conflict(_)));
        let status = reg.status();
        assert!(!status.active);
        assert!(status.last_error.is_some());
    }

    #[test]
    fn format_hint_renders_unbound_when_blank() {
        let bindings = HotkeyBindings::defaults();
        assert_eq!(
            format_hint(&bindings, HotkeyAction::RegionCapture),
            "Ctrl+Shift+S"
        );
        let empty = HotkeyBindings {
            region_capture: None,
            ..HotkeyBindings::defaults()
        };
        assert_eq!(format_hint(&empty, HotkeyAction::RegionCapture), "unbound");
    }

    #[test]
    fn status_to_dto_maps_action_id() {
        let status = HotkeyRegistryStatus {
            active: false,
            paused: false,
            last_error: Some("boom".to_string()),
            conflicting_action: Some(HotkeyAction::ShelfToggle),
        };
        let dto = status_to_dto(&status);
        assert_eq!(dto.conflicting_action.as_deref(), Some("shelf_toggle"));
    }

    #[test]
    fn validate_rejects_empty_string_with_action_label() {
        // validate is a pure helper so it doesn't need the
        // registry instance — both backends must accept it.
        let err = HotkeyRegistry::validate(HotkeyAction::RegionCapture, "").expect_err("empty");
        assert!(format!("{err:?}").contains("region_capture"));
    }
}
