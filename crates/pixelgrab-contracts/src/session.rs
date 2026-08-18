//! Capture-session lifecycle states. See the parent spec section "capture-session
//! lifecycle".

use serde::{Deserialize, Serialize};

/// High-level state of the capture session. New capture requests must not
/// interleave with an active session; the orchestrator transitions sequentially
/// and rejects out-of-order transitions with `InvalidSessionState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// No overlay is shown. The overlay is pre-built and hidden.
    Idle,
    /// Native capture is running against all monitor framebuffers.
    Capturing,
    /// Frame captured, overlay is about to be shown.
    Ready,
    /// User is selecting a region and/or editing annotations.
    Selecting,
    /// Committing/cancelling cleanup is in progress.
    Committing,
    /// Cleanup in progress (overlay being hidden, locks released).
    Cleanup,
}

impl SessionState {
    /// Whether the session is currently busy (not `Idle`).
    pub fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Returns the legal next states for the current state.
    pub fn allowed_next(&self) -> &'static [SessionState] {
        match self {
            Self::Idle => &[Self::Capturing],
            Self::Capturing => &[Self::Ready, Self::Cleanup],
            Self::Ready => &[Self::Selecting, Self::Cleanup],
            Self::Selecting => &[Self::Committing, Self::Cleanup],
            Self::Committing => &[Self::Cleanup],
            Self::Cleanup => &[Self::Idle],
        }
    }
}

/// A concrete state transition request. The orchestrator validates that the
/// target state is in `allowed_next()` before applying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTransition {
    /// Target state.
    pub to: SessionState,
    /// Monotonic reason code (e.g. user pressed Escape, IPC commit).
    pub reason: SessionTransitionReason,
}

/// Reason codes for session transitions. Stable for telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionTransitionReason {
    /// Capture requested via tray or shortcut.
    CaptureRequested,
    /// Native capture finished.
    CaptureComplete,
    /// Native capture failed.
    CaptureFailed,
    /// Overlay shown to the user.
    OverlayShown,
    /// User pressed Escape without an active selection.
    Cancelled,
    /// User pressed Enter.
    CommitRequested,
    /// User selected commit-to-clipboard-only.
    ClipboardOnlyRequested,
    /// User selected Save As.
    SaveAsRequested,
    /// Cleanup complete.
    CleanupComplete,
    /// Internal invariant violation forced a reset.
    Reset,
}
