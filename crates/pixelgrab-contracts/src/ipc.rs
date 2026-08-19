//! Typed IPC payloads exchanged between the Rust core and the Svelte frontend.
//!
//! The wire shape is duplicated in `src/lib/ipc/types.ts` and the contract
//! tests verify the two stay in sync.

use serde::{Deserialize, Serialize};

use crate::cache::{CacheEntryMetadata, LockOwner, ShelfId, ShelfPosition};
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
    /// Shelf entry id, if `to_shelf` was true and the commit succeeded.
    /// When the commit fails the field is `None`; the shelf never sees
    /// a card for a failed commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shelf_id: Option<String>,
    /// Path the PNG was written to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png_path: Option<String>,
    /// PNG byte length, for diagnostics.
    pub png_bytes: u64,
    /// Total cache-entry size on disk (PNG + bitmap + metadata +
    /// manifest). Populated only when the entry was published.
    #[serde(default)]
    pub size_bytes: u64,
    /// Wall-clock millis when the cache entry became durable.
    #[serde(default)]
    pub created_at_ms: i64,
}

/// Wire shape for `update_cache_metadata` IPC. The frontend sends the
/// shelf id and the new editable metadata; the Rust core rewrites the
/// `metadata.json` file atomically and updates the in-memory snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCacheMetadataRequest {
    /// Shelf card id whose metadata should be replaced.
    pub shelf_id: ShelfId,
    /// New metadata body.
    pub metadata: CacheEntryMetadata,
}

/// Wire shape for `dismiss_cache_entry` IPC. Removes a shelf card and
/// attempts to delete the underlying cache entry. The Rust core rejects
/// the dismissal when the entry has any active locks other than the
/// `Shelf` owner the dismissal itself releases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissCacheEntryRequest {
    /// Shelf card id to dismiss.
    pub shelf_id: ShelfId,
}

/// Wire shape for `dismiss_cache_entry` IPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissCacheEntryResponse {
    /// True when the entry was fully removed from the cache.
    pub removed: bool,
    /// Diagnostic string. One of `"removed"`, `"still_locked"`,
    /// `"unknown_shelf_id"`.
    pub reason: String,
}

/// Snapshot of the current shelf state returned by `get_shelf_snapshot`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShelfSnapshot {
    /// Most-recently-committed cache entry, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<crate::cache::CacheEntry>,
    /// Computed placement for the shelf window. Always populated when
    /// an entry is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<ShelfPosition>,
    /// Active lock owners on the current entry. Empty when no card is
    /// visible.
    #[serde(default)]
    pub locks: Vec<LockOwner>,
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

/// Wire shape for the `request_capture` response. Carries the capture
/// metadata and the structured diagnostics record for telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResponse {
    /// Capture resolution (DTO shape for the frontend).
    pub capture: CaptureResolutionDto,
    /// Diagnostics record for the capture. `None` if the orchestrator has
    /// not stamped a record yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<CaptureDiagnostics>,
}

/// Wire shape for the `request_overlay` response. Includes the snapshot
/// the UI uses to render and the diagnostics record (now with the
/// capture-to-overlay latency populated).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOverlayResult {
    /// Updated session snapshot after the overlay has acknowledged its
    /// selection.
    pub snapshot: SessionSnapshot,
    /// Diagnostics record with the overlay-visible timestamp stamped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<CaptureDiagnostics>,
}

/// Wire shape for the `request_cancel` response. The `action` field is a
/// stable string the frontend uses to drive the visual state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelOutcome {
    /// Stable action label - one of "selection_cleared", "session_cancelled",
    /// or "noop".
    pub action: String,
    /// Updated session snapshot after the cancel has been processed.
    pub snapshot: SessionSnapshot,
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

/// Structured capture diagnostics. Returned alongside the
/// `CaptureResolution` so the frontend can drive its loading spinner and the
/// telemetry layer can attribute latency without inspecting log lines.
///
/// The struct never includes captured pixel bytes, clipboard content, or
/// file paths outside the application cache root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDiagnostics {
    /// Stable capture id (matches `CaptureResolution::capture_id`).
    pub capture_id: String,
    /// Wall-clock millisecond timestamp at which the capture request started.
    pub capture_started_at_ms: i64,
    /// Wall-clock millisecond timestamp at which the capture pipeline returned.
    pub capture_completed_at_ms: i64,
    /// Elapsed milliseconds from request start to capture complete.
    pub capture_duration_ms: i64,
    /// Wall-clock millisecond timestamp at which the overlay became visible.
    pub overlay_visible_at_ms: Option<i64>,
    /// Total latency (capture complete -> overlay visible). Populated after
    /// the overlay is shown; None during the capturing phase.
    pub capture_to_overlay_ms: Option<i64>,
    /// Identifier of the monitor captured (or "virtual-desktop" if stitched).
    pub monitor_id: String,
    /// Resolution the capture pipeline reported.
    pub bounds: PhysicalBounds,
    /// Stable failure discriminant, if the capture failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
}

impl CaptureDiagnostics {
    /// Build a fresh diagnostics record at capture-request time.
    pub fn started(
        capture_id: impl Into<String>,
        monitor_id: impl Into<String>,
        bounds: PhysicalBounds,
        started_at_ms: i64,
    ) -> Self {
        Self {
            capture_id: capture_id.into(),
            capture_started_at_ms: started_at_ms,
            capture_completed_at_ms: 0,
            capture_duration_ms: 0,
            overlay_visible_at_ms: None,
            capture_to_overlay_ms: None,
            monitor_id: monitor_id.into(),
            bounds,
            failure_kind: None,
        }
    }

    /// Mark the capture as completed and compute the duration.
    pub fn completed(mut self, completed_at_ms: i64) -> Self {
        self.capture_completed_at_ms = completed_at_ms;
        self.capture_duration_ms = completed_at_ms.saturating_sub(self.capture_started_at_ms);
        self
    }

    /// Mark the overlay as visible and compute the capture-to-overlay
    /// latency.
    pub fn overlay_visible(mut self, overlay_at_ms: i64) -> Self {
        self.overlay_visible_at_ms = Some(overlay_at_ms);
        if self.capture_completed_at_ms > 0 {
            self.capture_to_overlay_ms =
                Some(overlay_at_ms.saturating_sub(self.capture_completed_at_ms));
        }
        self
    }

    /// Record a failure discriminant. Use only the categorical kind string
    /// (e.g. `"capture_unavailable"`) - never the raw error message.
    pub fn failed(mut self, kind: impl Into<String>) -> Self {
        self.failure_kind = Some(kind.into());
        self
    }
}
