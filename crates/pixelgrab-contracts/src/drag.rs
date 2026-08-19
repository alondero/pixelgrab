//! External drag-and-drop contract types.
//!
//! These types are the platform-neutral vocabulary for the OLE drag pipeline
//! that ships a shelf card to an external application (browsers, Electron
//! apps, Explorer, IDEs). The Windows side lives in
//! `src-tauri/src/platform/windows/drag.rs` and is hidden behind the
//! `PixelGrabPlatform::start_drag` trait method.
//!
//! See issue #21 (Tracer 09) and `docs/adr/0006-external-drag.md` for the
//! design rationale.

use serde::{Deserialize, Serialize};

use crate::PlatformError;

/// The terminal result of a drag operation. Order is significant for the
/// JSON wire shape; new variants append at the end.
///
/// The discriminant is stable across releases and is mirrored in
/// `src/lib/ipc/types.ts` and the shelf/card Svelte components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragOutcome {
    /// The drop target accepted the data. The card may be dismissed when
    /// the configured dismiss-on-drop policy is enabled.
    Accepted,
    /// The drop target refused the data (returned `DROPEFFECT_NONE` or
    /// rejected the offered formats). The card remains on the shelf.
    Rejected,
    /// The user released the drag outside any drop target or pressed
    /// Escape. The card remains on the shelf.
    Cancelled,
    /// The OLE drag pipeline itself failed (COM allocation, file handle,
    /// etc.). The card remains on the shelf and the error is reported
    /// through the diagnostics record.
    Failed,
}

impl DragOutcome {
    /// Whether the shelf card should be dismissed after this outcome.
    /// Only `Accepted` removes the card; the remaining outcomes are
    /// retryable by the user.
    pub fn dismiss_card(&self) -> bool {
        matches!(self, DragOutcome::Accepted)
    }
}

/// A clipboard format that a drop target may pull from the offering. The
/// set is the customary "good citizen" set for image drops: a file
/// reference (`CF_HDROP`), the registered PNG format Chromium/Edge use to
/// render directly, the legacy bitmap (`CF_DIBV5`), and a text fallback
/// that contains the absolute file path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragFormat {
    /// `CF_HDROP` — a file group descriptor pointing at the on-disk PNG.
    Hdrop,
    /// `CFSTR_FILEDESCRIPTOR`/`CFSTR_FILECONTENTS` — a registered PNG format
    /// that browsers and Electron apps consume directly.
    RegisteredPng,
    /// `CF_DIBV5` — a top-down BGRA bitmap with a V5 header.
    DibV5,
    /// `CF_UNICODETEXT` — the absolute PNG path as a UTF-16 wide string.
    /// Used by text-input targets that do not interpret the bitmap formats.
    UnicodeText,
}

impl DragFormat {
    /// Stable wire label used in diagnostics. Mirrors the serde
    /// snake_case output so log records and JSON payloads describe
    /// the same format with the same label.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Hdrop => "hdrop",
            Self::RegisteredPng => "registered_png",
            Self::DibV5 => "dib_v5",
            Self::UnicodeText => "unicode_text",
        }
    }
}

/// Whether the drop target requested the format during the drag loop.
/// Recorded for diagnostics — never logged with the captured content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragFormatRequest {
    /// Which format the target asked for.
    pub format: DragFormat,
    /// Wall-clock millisecond timestamp of the request (relative to the
    /// drag start).
    pub at_ms: i64,
}

/// Categorical drop-target effect. Stable label only — the discriminator
/// never includes process IDs, window handles, or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragTargetEffect {
    /// `DROPEFFECT_COPY` — the target copied the data.
    Copy,
    /// `DROPEFFECT_MOVE` — the target moved the data (treated as Copy for
    /// us; the shelf PNG is never deleted unilaterally).
    Move,
    /// `DROPEFFECT_NONE` — the target rejected the drop.
    None,
    /// The drag did not produce a target effect (e.g. cancelled before a
    /// target was reached).
    Unknown,
}

impl DragTargetEffect {
    /// Stable wire label.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Copy => "drop_copy",
            Self::Move => "drop_move",
            Self::None => "drop_none",
            Self::Unknown => "drop_unknown",
        }
    }
}

/// Categorical drop-target class. Used purely for diagnostics — the value
/// is a label, never a process path or title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragTargetKind {
    /// Chromium-based browser (Edge, Chrome, Brave, …).
    Chromium,
    /// Electron-hosted application.
    Electron,
    /// Windows Explorer (`explorer.exe`).
    Explorer,
    /// An IDE (VS Code, JetBrains, Visual Studio, …).
    Ide,
    /// A drop target that explicitly rejected every offered format.
    Rejecting,
    /// Any other drop target that accepted the drag.
    Other,
    /// No target was reached (the user cancelled).
    None,
}

impl DragTargetKind {
    /// Stable wire label.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Electron => "electron",
            Self::Explorer => "explorer",
            Self::Ide => "ide",
            Self::Rejecting => "rejecting",
            Self::Other => "other",
            Self::None => "none",
        }
    }
}

/// The structured diagnostics record for a single drag. Returned by the
/// platform contract alongside the terminal `DragOutcome`. The fields are
/// chosen so they are safe to log: they never include captured pixels,
/// annotation text, the absolute PNG path, or any user data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragDiagnostics {
    /// Wall-clock milliseconds when the drag started.
    pub started_at_ms: i64,
    /// Wall-clock milliseconds when the drag terminated.
    pub completed_at_ms: i64,
    /// Total drag duration in milliseconds.
    pub duration_ms: i64,
    /// Formats the drop target requested during the drag loop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_formats: Vec<DragFormatRequest>,
    /// Categorical drop target effect.
    pub target_effect: DragTargetEffect,
    /// Categorical drop target class.
    pub target_kind: DragTargetKind,
    /// Stable failure discriminant, present only when `DragOutcome::Failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    /// Stable capture id propagated to the cache. Never contains the PNG
    /// path.
    pub capture_id: String,
    /// Stable shelf id, when the drag originated from a shelf card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shelf_id: Option<String>,
}

impl DragDiagnostics {
    /// Build a fresh diagnostics record at drag-start time.
    pub fn started(
        capture_id: impl Into<String>,
        shelf_id: Option<String>,
        started_at_ms: i64,
    ) -> Self {
        Self {
            started_at_ms,
            completed_at_ms: 0,
            duration_ms: 0,
            requested_formats: Vec::new(),
            target_effect: DragTargetEffect::Unknown,
            target_kind: DragTargetKind::None,
            failure_kind: None,
            capture_id: capture_id.into(),
            shelf_id,
        }
    }

    /// Mark the drag as completed and compute the duration.
    pub fn completed(mut self, completed_at_ms: i64) -> Self {
        self.completed_at_ms = completed_at_ms;
        self.duration_ms = completed_at_ms.saturating_sub(self.started_at_ms);
        self
    }

    /// Record the terminal target effect.
    pub fn with_target_effect(mut self, effect: DragTargetEffect) -> Self {
        self.target_effect = effect;
        self
    }

    /// Record the categorical target class.
    pub fn with_target_kind(mut self, kind: DragTargetKind) -> Self {
        self.target_kind = kind;
        self
    }

    /// Record a format request from the drop target.
    pub fn record_format_request(&mut self, format: DragFormat, at_ms: i64) {
        self.requested_formats
            .push(DragFormatRequest { format, at_ms });
    }

    /// Record a failure discriminant. The kind is the categorical
    /// `PlatformErrorKind` label — never the raw error message.
    pub fn failed(mut self, kind: impl Into<String>) -> Self {
        self.failure_kind = Some(kind.into());
        self
    }
}

/// The composite result of a drag. Allocated as a single struct so the
/// platform contract has one return type and the IPC layer can project the
/// `outcome` into the typed response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragResult {
    /// Terminal outcome of the drag.
    pub outcome: DragOutcome,
    /// Diagnostics record. Always present so the IPC layer can surface the
    /// `failure_kind` and target effect through the typed event flow.
    pub diagnostics: DragDiagnostics,
}

/// The composed request sent to the platform contract. Carries the stable
/// PNG path the platform must keep on disk for the full synchronous OLE
/// loop, plus the cache correlation identifiers propagated into
/// `DragDiagnostics`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragRequest {
    /// Capture id of the underlying capture. Propagated into diagnostics
    /// for telemetry correlation.
    pub capture_id: String,
    /// Shelf id, when the drag originated from a shelf card. Optional
    /// because the IPC may also start a drag from a recent-capture
    /// shortcut that did not flow through the shelf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shelf_id: Option<String>,
    /// Absolute path to the stable PNG the platform must offer through
    /// `CF_HDROP`, the registered PNG format, and `CF_UNICODETEXT`. The
    /// platform contract must keep this file alive for the full drag
    /// operation and must not prune it during that window.
    pub png_path: String,
    /// Bitmap-compatible BGRA representation of the same image. The
    /// Windows platform contract uses this to back the `CF_DIBV5` format
    /// without re-decoding the PNG.
    pub bgra_pixels: Vec<u8>,
    /// Width in pixels of the bitmap representation.
    pub width: u32,
    /// Height in pixels of the bitmap representation.
    pub height: u32,
}

impl DragRequest {
    /// Validate that the request is internally consistent. The platform
    /// contract runs this check before allocating the OLE state.
    pub fn validate(&self) -> Result<(), PlatformError> {
        if self.png_path.is_empty() {
            return Err(PlatformError::new(
                crate::PlatformErrorKind::InvalidPayload,
                "drag request: png_path is empty",
            ));
        }
        if self.width == 0 || self.height == 0 {
            return Err(PlatformError::new(
                crate::PlatformErrorKind::InvalidPayload,
                "drag request: bitmap dimensions must be non-zero",
            ));
        }
        let expected = (self.width as usize) * (self.height as usize) * 4;
        if self.bgra_pixels.len() != expected {
            return Err(PlatformError::new(
                crate::PlatformErrorKind::InvalidPayload,
                format!(
                    "drag request: bgra buffer length {} does not match {}x{}x4",
                    self.bgra_pixels.len(),
                    self.width,
                    self.height,
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> DragRequest {
        DragRequest {
            capture_id: "capture-1".into(),
            shelf_id: Some("shelf-1".to_string()),
            png_path: "C:/cache/capture-1.png".into(),
            bgra_pixels: vec![0u8; 8 * 8 * 4],
            width: 8,
            height: 8,
        }
    }

    #[test]
    fn outcome_dismiss_card_only_for_accepted() {
        assert!(DragOutcome::Accepted.dismiss_card());
        assert!(!DragOutcome::Rejected.dismiss_card());
        assert!(!DragOutcome::Cancelled.dismiss_card());
        assert!(!DragOutcome::Failed.dismiss_card());
    }

    #[test]
    fn format_labels_are_stable() {
        assert_eq!(DragFormat::Hdrop.as_label(), "hdrop");
        assert_eq!(DragFormat::RegisteredPng.as_label(), "registered_png");
        assert_eq!(DragFormat::DibV5.as_label(), "dib_v5");
        assert_eq!(DragFormat::UnicodeText.as_label(), "unicode_text");
    }

    #[test]
    fn target_kind_labels_are_stable() {
        assert_eq!(DragTargetKind::Chromium.as_label(), "chromium");
        assert_eq!(DragTargetKind::Electron.as_label(), "electron");
        assert_eq!(DragTargetKind::Explorer.as_label(), "explorer");
        assert_eq!(DragTargetKind::Ide.as_label(), "ide");
        assert_eq!(DragTargetKind::Rejecting.as_label(), "rejecting");
        assert_eq!(DragTargetKind::Other.as_label(), "other");
        assert_eq!(DragTargetKind::None.as_label(), "none");
    }

    #[test]
    fn request_validation_rejects_empty_path() {
        let mut req = sample_request();
        req.png_path = String::new();
        assert!(req.validate().is_err());
    }

    #[test]
    fn request_validation_rejects_zero_dimensions() {
        let mut req = sample_request();
        req.width = 0;
        assert!(req.validate().is_err());
        req.width = 8;
        req.height = 0;
        assert!(req.validate().is_err());
    }

    #[test]
    fn request_validation_rejects_short_buffer() {
        let mut req = sample_request();
        req.bgra_pixels = vec![0u8; 7];
        assert!(req.validate().is_err());
    }

    #[test]
    fn request_validation_accepts_well_formed() {
        let req = sample_request();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn diagnostics_record_computes_duration() {
        let diag =
            DragDiagnostics::started("cap", Some("shelf".to_string()), 1_000).completed(1_250);
        assert_eq!(diag.duration_ms, 250);
        assert_eq!(diag.completed_at_ms, 1_250);
    }

    #[test]
    fn diagnostics_records_format_request() {
        let mut diag = DragDiagnostics::started("cap", None, 1_000);
        diag.record_format_request(DragFormat::Hdrop, 5);
        diag.record_format_request(DragFormat::DibV5, 12);
        assert_eq!(diag.requested_formats.len(), 2);
        assert_eq!(diag.requested_formats[0].format, DragFormat::Hdrop);
        assert_eq!(diag.requested_formats[1].at_ms, 12);
    }

    #[test]
    fn diagnostics_failure_is_optional() {
        let diag = DragDiagnostics::started("cap", None, 1_000)
            .completed(1_000)
            .with_target_effect(DragTargetEffect::None)
            .with_target_kind(DragTargetKind::Rejecting);
        assert!(diag.failure_kind.is_none());
        let diag = diag.failed("io");
        assert_eq!(diag.failure_kind.as_deref(), Some("io"));
    }

    #[test]
    fn result_serialises_outcome_only() {
        let diag =
            DragDiagnostics::started("cap", Some("shelf".to_string()), 1_000).completed(1_250);
        let result = DragResult {
            outcome: DragOutcome::Accepted,
            diagnostics: diag,
        };
        let json = serde_json::to_string(&result).expect("serialise");
        assert!(json.contains("\"outcome\":\"accepted\""));
        assert!(json.contains("\"durationMs\":250"));
        assert!(json.contains("\"captureId\":\"cap\""));
    }
}
