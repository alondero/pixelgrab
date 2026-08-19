//! Tauri command handlers. Each command is a thin wrapper that translates
//! the WebView payload into the platform capture flow and returns the typed
//! result.

use std::time::{SystemTime, UNIX_EPOCH};

use pixelgrab_contracts::{
    capture::{CaptureFormat, CaptureRequest},
    ipc::{
        CancelOutcome, CaptureResponse, CommitRequest, CommitResponse, IpcResponse,
        RequestCaptureIntent, RequestCommitIntent, RequestOverlayIntent, RequestOverlayResult,
        SessionSnapshot,
    },
    CaptureDiagnostics, PlatformError, PlatformErrorKind,
};
use tauri::State;

use crate::PixelGrabApp;

type AppState<'a> = State<'a, PixelGrabApp>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Begin a capture from the tray or shortcut. The orchestrator refuses the
/// call when the session is already busy; an overlapping capture request
/// cannot replace or corrupt the in-flight session.
#[tauri::command]
pub fn request_capture(
    app: AppState<'_>,
    payload: RequestCaptureIntent,
) -> IpcResponse<CaptureResponse> {
    let _ = payload; // shape-only payload; the actual format comes from the
                     // orchestrator's platform contract.
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let started_at = now_ms();
    let result = app.session().request_capture(&request);
    let capture = match result {
        Ok(capture) => capture,
        Err(err) => {
            // Record a failure diagnostic and return the error.
            let diag = CaptureDiagnostics::started(
                "<rejected>",
                "virtual-desktop",
                pixelgrab_contracts::PhysicalBounds::EMPTY,
                started_at,
            )
            .completed(now_ms())
            .failed(format!("{:?}", err.kind));
            app.session().store_diagnostics(diag);
            return IpcResponse::from_result(Err(err));
        }
    };
    let monitor_id = monitor_id_for(&capture);
    let diag =
        CaptureDiagnostics::started(&capture.capture_id, &monitor_id, capture.bounds, started_at)
            .completed(capture.captured_at_ms);
    app.session().store_diagnostics(diag);
    let response = CaptureResponse {
        capture: pixelgrab_contracts::ipc::CaptureResolutionDto::from(capture),
        diagnostics: app.session().last_diagnostics(),
    };
    IpcResponse::from_result(Ok(response))
}

/// The overlay calls this when it has finished showing the freeze frame.
/// Transitions to `Selecting` and stamps the overlay-visible timestamp
/// onto the stored diagnostics record.
#[tauri::command]
pub fn request_overlay(
    app: AppState<'_>,
    payload: RequestOverlayIntent,
) -> IpcResponse<RequestOverlayResult> {
    let result = match app.session().overlay_visible() {
        Ok(()) => app
            .session()
            .report_selection(payload.selection)
            .map(|_| RequestOverlayResult {
                snapshot: app.session().snapshot(),
                diagnostics: app.session().last_diagnostics(),
            }),
        Err(err) => Err(err),
    };
    IpcResponse::from_result(result)
}

/// Cancel the active session. Honours the staged Escape behaviour: the
/// first call clears the selection if one is active; the second call
/// cancels the session and returns the overlay to the pool.
#[tauri::command]
pub fn request_cancel(app: AppState<'_>) -> IpcResponse<CancelOutcome> {
    let action = match app.session().handle_escape() {
        Ok(action) => action,
        Err(err) => return IpcResponse::from_result(Err(err)),
    };
    let outcome = match action {
        crate::session::state::EscapeAction::SelectionCleared => CancelOutcome {
            action: "selection_cleared".into(),
            snapshot: app.session().snapshot(),
        },
        crate::session::state::EscapeAction::SessionCancelled => CancelOutcome {
            action: "session_cancelled".into(),
            snapshot: app.session().snapshot(),
        },
        crate::session::state::EscapeAction::NoOp => CancelOutcome {
            action: "noop".into(),
            snapshot: app.session().snapshot(),
        },
    };
    IpcResponse::from_result(Ok(outcome))
}

/// Commit the current selection. Returns the commit outcome. The flattened
/// crop is the single source from which both the on-disk PNG and the
/// clipboard bitmap representation are derived.
#[tauri::command]
pub fn request_commit(
    app: AppState<'_>,
    payload: RequestCommitIntent,
) -> IpcResponse<CommitResponse> {
    let commit_request = CommitRequest {
        crop: payload.crop,
        to_shelf: payload.to_shelf,
        to_clipboard: payload.to_clipboard,
        save_as: payload.save_as,
    };
    let result = match commit(&app, &commit_request) {
        Ok(outcome) => Ok(CommitResponse { outcome }),
        Err(err) => Err(err),
    };
    IpcResponse::from_result(result)
}

/// Read the current session snapshot.
#[tauri::command]
pub fn get_session_snapshot(app: AppState<'_>) -> IpcResponse<SessionSnapshot> {
    IpcResponse::from_result(Ok(app.session().snapshot()))
}

fn commit(
    app: &PixelGrabApp,
    request: &CommitRequest,
) -> Result<pixelgrab_contracts::ipc::CommitOutcome, PlatformError> {
    use pixelgrab_contracts::ipc::CommitOutcome;
    let bbox = request.crop;
    bbox.validate().map_err(|_| {
        PlatformError::new(PlatformErrorKind::InvalidPayload, "empty commit bounds")
    })?;

    // Resolve the capture_id of the frame we are about to flatten. The
    // overlay's report carries the physical bounds; the platform adapter
    // owns the frozen framebuffer that holds the matching pixels.
    let capture_id = app
        .session()
        .last_capture()
        .map(|c| c.capture_id)
        .ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::InvalidSessionState,
                "no active capture to commit",
            )
        })?;

    // Single source of truth: flatten the crop once. The PNG bytes and the
    // clipboard bitmap are both derived from this buffer.
    let (rgba, size) = app.platform().flatten_crop(&capture_id, bbox)?;

    let mut outcome = CommitOutcome {
        capture_id: capture_id.clone(),
        shelf_id: None,
        png_path: None,
        png_bytes: 0,
    };

    if request.to_shelf || request.save_as {
        let path = app.platform().write_png(&capture_id, bbox, &rgba)?;
        outcome.png_path = Some(path.to_string_lossy().to_string());
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        outcome.png_bytes = bytes;
    } else {
        outcome.png_bytes = rgba.len() as u64;
    }

    if request.to_clipboard {
        // The clipboard path is best-effort - if the platform has no
        // clipboard (synthetic CI) the call returns Ok without doing
        // anything, so the commit still succeeds.
        app.platform().publish_clipboard(&capture_id, &rgba, size)?;
    }

    if let Err(err) = app.session().finish() {
        log::warn!("session.finish failed: {err}");
    }
    Ok(outcome)
}

/// Derive a diagnostic monitor id from a capture resolution. The label is
/// stable across releases and never embeds user data; it is purely a
/// categorical identifier for telemetry.
fn monitor_id_for(capture: &pixelgrab_contracts::capture::CaptureResolution) -> String {
    use pixelgrab_contracts::capture::CaptureFormat;
    match capture.format {
        CaptureFormat::VirtualDesktop => "virtual-desktop".into(),
        CaptureFormat::SingleMonitor => "single-monitor".into(),
        CaptureFormat::PhysicalRegion => "physical-region".into(),
    }
}
