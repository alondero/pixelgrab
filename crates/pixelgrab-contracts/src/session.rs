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
    /// A shelf entry is loaded into the editor for non-destructive
    /// editing. Tracer-10. The source entry holds a `Shelf` +
    /// `Editor` lock pair; the original card stays on the shelf.
    /// A new capture request is rejected with `InvalidSessionState`,
    /// matching the existing overlap guard.
    Reopening,
    /// A revision commit is in flight (the user accepted the edit).
    /// Same overlap guard as `Committing`: any second capture or
    /// second revision commit is rejected.
    RevisionCommitting,
}

impl SessionState {
    /// Whether the session is currently busy (not `Idle`).
    pub fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Returns the legal next states for the current state.
    pub fn allowed_next(&self) -> &'static [SessionState] {
        match self {
            Self::Idle => &[Self::Capturing, Self::Reopening],
            Self::Capturing => &[Self::Ready, Self::Cleanup],
            Self::Ready => &[Self::Selecting, Self::Cleanup],
            Self::Selecting => &[Self::Committing, Self::Cleanup],
            Self::Committing => &[Self::Cleanup],
            Self::Cleanup => &[Self::Idle],
            Self::Reopening => &[Self::RevisionCommitting, Self::Cleanup, Self::Idle],
            Self::RevisionCommitting => &[Self::Cleanup, Self::Idle],
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
    /// User directed the resident process to reopen a shelf entry.
    /// Tracer-10. Transitions `Idle -> Reopening`.
    ReopenRequested,
    /// User pressed Enter on a reopened shelf entry (commit the
    /// revision). Tracer-10. Transitions `Reopening -> RevisionCommitting`.
    RevisionCommitRequested,
    /// User cancelled the reopen session (Escape). Tracer-10.
    /// Transitions `Reopening -> Idle`.
    RevisionCancelled,
}
