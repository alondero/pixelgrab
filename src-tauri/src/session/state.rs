//! Session state machine. The orchestrator is the only place that mutates
//! `SessionState`. All trigger points (tray, IPC, shortcuts) go through the
//! `request_transition` method.

use std::sync::Arc;

use pixelgrab_contracts::{
    capture::{CaptureRequest, CaptureResolution},
    coordinate::PhysicalBounds,
    ipc::SessionSnapshot,
    session::{SessionState, SessionTransition, SessionTransitionReason},
    PlatformError, PlatformErrorKind, PlatformResult,
};

use crate::platform::PixelGrabPlatform;

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
    selection: Option<PhysicalBounds>,
}

impl SessionOrchestrator {
    /// Build a new orchestrator bound to the given platform contract.
    pub fn new(platform: Arc<dyn PixelGrabPlatform>) -> Self {
        Self {
            inner: parking_lot::Mutex::new(SessionInner {
                state: SessionState::Idle,
                last_capture: None,
                selection: None,
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

    /// Run a synthetic capture pipeline either directly or via the platform
    /// contract and update the session state. Used by the synthetic
    /// end-to-end trace and by tests.
    pub fn run_capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        self.request_transition(SessionTransition {
            to: SessionState::Capturing,
            reason: SessionTransitionReason::CaptureRequested,
        })?;
        let capture = self.platform.capture(request)?;
        {
            let mut inner = self.inner.lock();
            inner.last_capture = Some(capture.clone());
        }
        self.request_transition(SessionTransition {
            to: SessionState::Ready,
            reason: SessionTransitionReason::CaptureComplete,
        })?;
        Ok(capture)
    }

    /// Move the overlay into the selecting state.
    pub fn begin_selecting(&self) -> PlatformResult<()> {
        self.request_transition(SessionTransition {
            to: SessionState::Selecting,
            reason: SessionTransitionReason::OverlayShown,
        })
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
}
