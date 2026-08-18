//! Typed IPC payloads exchanged between the Rust core and the Svelte frontend.
//!
//! The wire shape is duplicated in `src/lib/ipc/types.ts` and the contract
//! tests verify the two stay in sync.

use serde::{Deserialize, Serialize};

use crate::capture::CaptureResolution;
use crate::coordinate::PhysicalBounds;
use crate::error::PlatformError;
use crate::session::SessionState;

/// Frontend-friendly DTO mirroring `CaptureResolution`. Used as the wire
/// shape for the `request_capture` response; the field names match the
/// TypeScript declaration in `src/lib/ipc/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResolutionDto {
    /// Format used.
    pub format: String,
    /// Physical bounds.
    pub bounds: PhysicalBounds,
    /// Asset URL.
    pub asset_url: String,
    /// Capture id.
    pub capture_id: String,
    /// Timestamp.
    pub captured_at_ms: i64,
}

impl From<CaptureResolution> for CaptureResolutionDto {
    fn from(c: CaptureResolution) -> Self {
        Self {
            format: format!("{:?}", c.format).to_lowercase(),
            bounds: c.bounds,
            asset_url: c.asset_url,
            capture_id: c.capture_id,
            captured_at_ms: c.captured_at_ms,
        }
    }
}

/// Wire shape for the response to `request_commit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResponse {
    /// The commit outcome.
    pub outcome: CommitOutcome,
}

/// Wrapper for typed IPC responses. The error variant is the wire shape for
/// `PlatformError` and the success variant carries a payload type.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IpcResponse<T> {
    /// Success.
    Ok {
        /// Payload.
        data: T,
    },
    /// Failure.
    Err {
        /// Wire-shaped error.
        error: PlatformError,
    },
}

impl<T> IpcResponse<T> {
    /// Convert a `Result` into the wire shape.
    pub fn from_result(result: Result<T, PlatformError>) -> Self {
        match result {
            Ok(data) => Self::Ok { data },
            Err(error) => Self::Err { error },
        }
    }
}

/// IPC error sentinel used for protocol-level errors (e.g. unknown command).
/// Distinct from [`PlatformError`] which describes application failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IpcError {
    /// The command name is not registered.
    UnknownCommand,
    /// The payload could not be deserialised.
    BadPayload,
    /// The command exists but is not enabled in this build (e.g. a Linux stub).
    NotAvailable,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::UnknownCommand => "unknown command",
            Self::BadPayload => "bad payload",
            Self::NotAvailable => "not available",
        };
        f.write_str(label)
    }
}

impl std::error::Error for IpcError {}

/// User-driven intent captured by the tray or global shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureIntent {
    /// Capture a region the user selects on the overlay.
    Region,
    /// Capture the full virtual desktop.
    FullScreen,
}

/// Wire shape for `RequestCapture` IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestCaptureIntent {
    /// Which capture to initiate.
    pub intent: CaptureIntent,
}

/// Wire shape for `RequestOverlay` IPC - the overlay tells the Rust core
/// what the user selected and asks for a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOverlayIntent {
    /// Physical-pixel selection. Empty bounds cancel the session.
    pub selection: PhysicalBounds,
}

/// Wire shape for `RequestCommit` IPC - the frontend confirms the commit
/// policy (clipboard, shelf, save-as) and the Rust core returns the
/// `CommitOutcome`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestCommitIntent {
    /// Final physical crop.
    pub crop: PhysicalBounds,
    /// Whether to retain the capture on the shelf.
    pub to_shelf: bool,
    /// Whether to copy the flattened PNG to the clipboard.
    pub to_clipboard: bool,
    /// Whether to invoke the native Save As dialog.
    #[serde(default)]
    pub save_as: bool,
}

/// Wire shape for `RequestCommit` IPC - the Rust core confirms the commit
/// policy and returns the outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRequest {
    /// Final physical crop.
    pub crop: PhysicalBounds,
    /// Whether to retain the capture on the shelf.
    pub to_shelf: bool,
    /// Whether to copy the flattened PNG to the clipboard.
    pub to_clipboard: bool,
    /// Whether to invoke the native Save As dialog.
    #[serde(default)]
    pub save_as: bool,
}

/// Outcome of a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitOutcome {
    /// Capture id (UUID v4) assigned by the Rust core.
    pub capture_id: String,
    /// Shelf entry id, if `to_shelf` was true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shelf_id: Option<String>,
    /// Path the PNG was written to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png_path: Option<String>,
    /// PNG byte length, for diagnostics.
    pub png_bytes: u64,
}

/// A snapshot of the current session state for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    /// Current session state.
    pub state: SessionState,
    /// Last capture resolution, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_capture: Option<CaptureResolution>,
    /// Selection the overlay most recently reported, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<PhysicalBounds>,
}

/// The overlay's view of the user's selection. Mirrors `RequestOverlayIntent`
/// but is the *outcome* sent back to the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySelection {
    /// Final physical crop.
    pub crop: PhysicalBounds,
    /// Whether the user confirmed the selection (vs cancelled).
    pub confirmed: bool,
}
