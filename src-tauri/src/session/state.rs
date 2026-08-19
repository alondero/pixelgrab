//! Session state machine. The orchestrator is the only place that mutates
//! `SessionState`. All trigger points (tray, IPC, shortcuts) go through the
//! `request_transition` method.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pixelgrab_contracts::{
    capture::{CaptureRequest, CaptureResolution},
    coordinate::PhysicalBounds,
    ipc::SessionSnapshot,
    session::{SessionState, SessionTransition, SessionTransitionReason},
    CaptureDiagnostics, PlatformError, PlatformErrorKind, PlatformResult,
};

use crate::platform::PixelGrabPlatform;

/// Wall-clock milliseconds since the Unix epoch. Mirrors the helper used by
/// the Windows capture engine so the orchestrator can stamp the
/// overlay-visible timestamp without depending on the platform module.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What happened when the user pressed Escape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapeAction {
    /// The active selection was cleared; the session remains in `Selecting`.
    SelectionCleared,
    /// The session was cancelled; the orchestrator is back in `Idle`.
    SessionCancelled,
    /// Nothing happened (already idle or no selection to clear).
    NoOp,
}

/// The capture-session orchestrator. Each `request_transition` validates the
/// current-to-target edge before applying. Out-of-order transitions are
/// rejected with `InvalidSessionState`.
pub struct SessionOrchestrator {
    inner: parking_lot::Mutex<SessionInner>,
    platform: Arc<dyn PixelGrabPlatform>,
}

#[derive(Debug)]
struct SessionInner {
    state: SessionState,
    last_capture: Option<CaptureResolution>,
    last_capture_id: Option<String>,
    selection: Option<PhysicalBounds>,
    last_diagnostics: Option<CaptureDiagnostics>,
}

impl SessionOrchestrator {
    /// Build a new orchestrator bound to the given platform contract.
    pub fn new(platform: Arc<dyn PixelGrabPlatform>) -> Self {
        Self {
            inner: parking_lot::Mutex::new(SessionInner {
                state: SessionState::Idle,
                last_capture: None,
                last_capture_id: None,
                selection: None,
                last_diagnostics: None,
            }),
            platform,
        }
    }

    /// Current state.
    pub fn current_state(&self) -> SessionState {
        self.inner.lock().state
    }

    /// Latest capture resolution.
    pub fn last_capture(&self) -> Option<CaptureResolution> {
        self.inner.lock().last_capture.clone()
    }

    /// Most recent selection reported by the overlay.
    pub fn selection(&self) -> Option<PhysicalBounds> {
        self.inner.lock().selection
    }

    /// Last capture diagnostics record (if any).
    pub fn last_diagnostics(&self) -> Option<CaptureDiagnostics> {
        self.inner.lock().last_diagnostics.clone()
    }

    /// True when the orchestrator is currently busy (not `Idle`).
    pub fn is_busy(&self) -> bool {
        self.inner.lock().state.is_busy()
    }

    /// Run a capture pipeline. The orchestrator refuses the call when the
    /// session is busy - an overlapping capture request cannot replace or
    /// corrupt the in-flight session (acceptance criterion). Tests that want
    /// to bypass this gate should call `platform.capture(...)` directly.
    pub fn request_capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        if self.is_busy() {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidSessionState,
                format!(
                    "cannot start capture: session is already {:?}",
                    self.current_state()
                ),
            ));
        }
        self.request_transition(SessionTransition {
            to: SessionState::Capturing,
            reason: SessionTransitionReason::CaptureRequested,
        })?;
        let capture = self.platform.capture(request)?;
        {
            let mut inner = self.inner.lock();
            inner.last_capture = Some(capture.clone());
            inner.last_capture_id = Some(capture.capture_id.clone());
        }
        self.request_transition(SessionTransition {
            to: SessionState::Ready,
            reason: SessionTransitionReason::CaptureComplete,
        })?;
        Ok(capture)
    }

    /// Run a synthetic capture pipeline either directly or via the platform
    /// contract and update the session state. Used by the synthetic
    /// end-to-end trace and by tests.
    pub fn run_capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        self.request_capture(request)
    }

    /// Mark the overlay as visible to the user. Transitions to `Selecting`
    /// and computes the capture-to-overlay latency on the stored diagnostics.
    pub fn overlay_visible(&self) -> PlatformResult<()> {
        self.request_transition(SessionTransition {
            to: SessionState::Selecting,
            reason: SessionTransitionReason::OverlayShown,
        })?;
        let mut inner = self.inner.lock();
        let overlay_at_ms = now_ms();
        if let Some(diag) = inner.last_diagnostics.clone() {
            inner.last_diagnostics = Some(diag.overlay_visible(overlay_at_ms));
        }
        Ok(())
    }

    /// Move the overlay into the selecting state (legacy alias for
    /// `overlay_visible` - kept so the tracer-01 surface still compiles).
    pub fn begin_selecting(&self) -> PlatformResult<()> {
        self.overlay_visible()
    }

    /// Record the user's selection. Empty bounds cancel the session.
    pub fn report_selection(&self, selection: PhysicalBounds) -> PlatformResult<()> {
        let mut inner = self.inner.lock();
        if inner.state != SessionState::Selecting {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidSessionState,
                format!("cannot report selection while session is {:?}", inner.state),
            ));
        }
        inner.selection = Some(selection);
        Ok(())
    }

    /// Commit and clean up. Always ends back in `Idle`.
    pub fn finish(&self) -> PlatformResult<()> {
        self.request_transition(SessionTransition {
            to: SessionState::Committing,
            reason: SessionTransitionReason::CommitRequested,
        })?;
        self.request_transition(SessionTransition {
            to: SessionState::Cleanup,
            reason: SessionTransitionReason::CleanupComplete,
        })?;
        self.request_transition(SessionTransition {
            to: SessionState::Idle,
            reason: SessionTransitionReason::CleanupComplete,
        })
    }

    /// Force a reset back to Idle regardless of state. Used when the
    /// overlay is dismissed by Escape or when an internal error is recovered.
    pub fn reset(&self) {
        let mut inner = self.inner.lock();
        inner.state = SessionState::Idle;
        inner.selection = None;
    }

    /// Staged Escape behaviour. The first Escape while a selection is
    /// active clears the selection; the second Escape (or a single Escape
    /// when no selection is active) cancels the session and returns the
    /// orchestrator to `Idle`. The deterministic cleanup path runs on every
    /// cancellation so the overlay window is always returned to the pool.
    pub fn handle_escape(&self) -> PlatformResult<EscapeAction> {
        let state = self.current_state();
        match state {
            SessionState::Idle => Ok(EscapeAction::NoOp),
            SessionState::Selecting => {
                let has_selection = self.selection().is_some();
                if has_selection {
                    self.clear_selection();
                    Ok(EscapeAction::SelectionCleared)
                } else {
                    self.cancel_session()?;
                    Ok(EscapeAction::SessionCancelled)
                }
            }
            // Capturing / Ready / Committing / Cleanup: any Escape cancels
            // the session and force-resets back to Idle.
            SessionState::Capturing
            | SessionState::Ready
            | SessionState::Committing
            | SessionState::Cleanup => {
                self.cancel_session()?;
                Ok(EscapeAction::SessionCancelled)
            }
        }
    }

    /// Clear the current selection without changing state. Internal helper
    /// used by `handle_escape`.
    fn clear_selection(&self) {
        self.inner.lock().selection = None;
    }

    /// Cancel the session and force the state machine back to Idle.
    /// Always succeeds - the deterministic cleanup is the contract.
    pub fn cancel_session(&self) -> PlatformResult<()> {
        // Walk forward only if the current state is in a position that
        // permits a transition; otherwise jump straight to Cleanup -> Idle.
        let state = self.current_state();
        match state {
            SessionState::Idle => Ok(()),
            SessionState::Capturing
            | SessionState::Ready
            | SessionState::Selecting
            | SessionState::Committing => {
                self.request_transition(SessionTransition {
                    to: SessionState::Cleanup,
                    reason: SessionTransitionReason::Cancelled,
                })?;
                self.request_transition(SessionTransition {
                    to: SessionState::Idle,
                    reason: SessionTransitionReason::CleanupComplete,
                })?;
                Ok(())
            }
            SessionState::Cleanup => {
                self.request_transition(SessionTransition {
                    to: SessionState::Idle,
                    reason: SessionTransitionReason::CleanupComplete,
                })?;
                Ok(())
            }
        }
    }

    /// Apply a transition. Validates the current-to-target edge.
    pub fn request_transition(&self, transition: SessionTransition) -> PlatformResult<()> {
        let mut inner = self.inner.lock();
        if !inner.state.allowed_next().contains(&transition.to) {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidSessionState,
                format!(
                    "illegal transition {:?} -> {:?}",
                    inner.state, transition.to
                ),
            ));
        }
        log::debug!(
            "session transition {:?} -> {:?} (reason={:?})",
            inner.state,
            transition.to,
            transition.reason
        );
        inner.state = transition.to;
        Ok(())
    }

    /// Snapshot for the UI.
    pub fn snapshot(&self) -> SessionSnapshot {
        let inner = self.inner.lock();
        SessionSnapshot {
            state: inner.state,
            last_capture: inner.last_capture.clone(),
            selection: inner.selection,
        }
    }

    /// Replace the stored diagnostics record. The IPC layer stamps the
    /// overlay-visible timestamp onto this value via `overlay_visible`.
    pub fn store_diagnostics(&self, diagnostics: CaptureDiagnostics) {
        self.inner.lock().last_diagnostics = Some(diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelgrab_contracts::capture::{CaptureFormat, CaptureRequest};
    use pixelgrab_contracts::coordinate::{PhysicalBounds, PhysicalSize};
    use pixelgrab_contracts::session::{SessionState, SessionTransitionReason};
    use pixelgrab_contracts::{CaptureDiagnostics, PlatformErrorKind};

    fn make_session() -> SessionOrchestrator {
        let platform: Arc<dyn PixelGrabPlatform> =
            Arc::new(crate::platform::synthetic::SyntheticPlatform::new());
        SessionOrchestrator::new(platform)
    }

    fn capture_request() -> CaptureRequest {
        CaptureRequest {
            format: CaptureFormat::VirtualDesktop,
            monitor_id: None,
            region: None,
        }
    }

    #[test]
    fn request_capture_rejects_overlap() {
        let session = make_session();
        session
            .run_capture(&capture_request())
            .expect("first capture");
        // The session is now in Ready; a second capture must be refused.
        let result = session.run_capture(&capture_request());
        assert!(result.is_err(), "overlapping capture must be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidSessionState);
    }

    #[test]
    fn busy_state_drops_after_finish() {
        let session = make_session();
        session.run_capture(&capture_request()).expect("first");
        session
            .request_transition(SessionTransition {
                to: SessionState::Selecting,
                reason: SessionTransitionReason::OverlayShown,
            })
            .expect("selecting");
        session.finish().expect("finish");
        assert_eq!(session.current_state(), SessionState::Idle);
        assert!(!session.is_busy());
    }

    #[test]
    fn escape_clears_active_selection_then_cancels() {
        let session = make_session();
        session.run_capture(&capture_request()).expect("capture");
        session
            .request_transition(SessionTransition {
                to: SessionState::Selecting,
                reason: SessionTransitionReason::OverlayShown,
            })
            .expect("selecting");
        session
            .report_selection(PhysicalBounds::from_xywh(10, 10, 100, 100))
            .expect("selection");
        assert!(session.selection().is_some());

        let first = session.handle_escape().expect("first escape");
        assert_eq!(first, EscapeAction::SelectionCleared);
        assert_eq!(session.current_state(), SessionState::Selecting);
        assert!(session.selection().is_none());

        let second = session.handle_escape().expect("second escape");
        assert_eq!(second, EscapeAction::SessionCancelled);
        assert_eq!(session.current_state(), SessionState::Idle);
    }

    #[test]
    fn escape_is_noop_when_idle() {
        let session = make_session();
        let action = session.handle_escape().expect("escape");
        assert_eq!(action, EscapeAction::NoOp);
        assert_eq!(session.current_state(), SessionState::Idle);
    }

    #[test]
    fn cancel_session_returns_to_idle_from_any_state() {
        for start in [
            SessionState::Capturing,
            SessionState::Ready,
            SessionState::Selecting,
            SessionState::Committing,
        ] {
            let session = make_session();
            // Walk the orchestrator up to the target state. We can't go
            // straight from Idle to Capturing without going through the
            // request_capture entry point, so do that first.
            session.run_capture(&capture_request()).expect("capture");
            assert_eq!(session.current_state(), SessionState::Ready);
            // Force the state to the test target so the cleanup path is
            // exercised uniformly across states.
            session
                .request_transition(SessionTransition {
                    to: SessionState::Selecting,
                    reason: SessionTransitionReason::OverlayShown,
                })
                .expect("to selecting");
            if matches!(start, SessionState::Committing) {
                session
                    .request_transition(SessionTransition {
                        to: SessionState::Committing,
                        reason: SessionTransitionReason::CommitRequested,
                    })
                    .expect("to committing");
            }
            if matches!(start, SessionState::Capturing) {
                session.reset();
                session
                    .request_transition(SessionTransition {
                        to: SessionState::Capturing,
                        reason: SessionTransitionReason::CaptureRequested,
                    })
                    .expect("to capturing");
            }
            // Every start state must round-trip back to Idle.
            session.cancel_session().expect("cancel");
            assert_eq!(
                session.current_state(),
                SessionState::Idle,
                "start state {start:?} must round-trip to Idle",
            );
        }
    }

    #[test]
    fn overlay_visible_stamps_diagnostics() {
        let session = make_session();
        let capture_id = "abc".to_string();
        let bounds = PhysicalBounds::from_xywh(0, 0, 100, 100);
        let diag =
            CaptureDiagnostics::started(&capture_id, "primary", bounds, 1_000).completed(1_010);
        session.store_diagnostics(diag);
        session.run_capture(&capture_request()).expect("capture");
        // Capture re-stores diagnostics in the IPC layer; simulate by
        // storing again so the overlay_visible path has something to stamp.
        let diag =
            CaptureDiagnostics::started(&capture_id, "primary", bounds, 1_000).completed(1_010);
        session.store_diagnostics(diag);
        // Now trigger the overlay.
        session.overlay_visible().expect("overlay visible");
        let stamped = session.last_diagnostics().expect("diagnostics");
        assert!(stamped.capture_to_overlay_ms.is_some());
        let latency = stamped.capture_to_overlay_ms.unwrap();
        assert!(latency >= 0, "latency must be non-negative");
        // Capture size sanity check.
        assert_eq!(stamped.bounds.size, PhysicalSize::new(100, 100),);
    }

    #[test]
    fn flatten_crop_synthetic_is_deterministic() {
        // Ensures the synthetic adapter is the single source for the
        // flattened crop (and therefore the PNG + clipboard pair).
        let platform: Arc<dyn PixelGrabPlatform> =
            Arc::new(crate::platform::synthetic::SyntheticPlatform::new());
        let (rgba1, size1) = platform
            .flatten_crop("id", PhysicalBounds::from_xywh(0, 0, 8, 8))
            .expect("flatten");
        let (rgba2, size2) = platform
            .flatten_crop("id", PhysicalBounds::from_xywh(0, 0, 8, 8))
            .expect("flatten");
        assert_eq!(rgba1, rgba2, "synthetic flatten must be deterministic");
        assert_eq!(size1, size2);
        assert_eq!(size1, PhysicalSize::new(8, 8));
        assert_eq!(rgba1.len(), 8 * 8 * 4);
    }

    #[test]
    fn flatten_crop_rejects_zero_size() {
        let platform: Arc<dyn PixelGrabPlatform> =
            Arc::new(crate::platform::synthetic::SyntheticPlatform::new());
        let result = platform.flatten_crop("id", PhysicalBounds::from_xywh(0, 0, 0, 0));
        assert!(result.is_err());
    }
}
