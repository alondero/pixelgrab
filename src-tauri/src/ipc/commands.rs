//! Tauri command handlers. Each command is a thin wrapper that translates
//! the WebView payload into a synthetic-capture flow and returns the typed
//! result.

use pixelgrab_contracts::{
    capture::{CaptureFormat, CaptureRequest},
    ipc::{
        CommitRequest, CommitResponse, IpcResponse, RequestCaptureIntent, RequestCommitIntent,
        RequestOverlayIntent, SessionSnapshot,
    },
    PlatformError, PlatformErrorKind,
};
use tauri::State;

use crate::PixelGrabApp;

type AppState<'a> = State<'a, PixelGrabApp>;

/// Begin a capture from the tray or shortcut. This is the entry point for the
/// "tray intent -> Rust IPC" leg of the synthetic end-to-end trace.
#[tauri::command]
pub fn request_capture(
    app: AppState<'_>,
    payload: RequestCaptureIntent,
) -> IpcResponse<pixelgrab_contracts::ipc::CaptureResolutionDto> {
    // The synthetic platform only emits the request via the orchestrator and
    // then exposes the resolution as a DTO. The full capture request is
    // always issued against the virtual desktop for tracer-01.
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let _ = payload; // captured by request
    let result = match app.session().run_capture(&request) {
        Ok(capture) => Ok(pixelgrab_contracts::ipc::CaptureResolutionDto::from(
            capture,
        )),
        Err(err) => Err(err),
    };
    IpcResponse::from_result(result)
}

/// The overlay calls this when the user has finished selecting a region.
#[tauri::command]
pub fn request_overlay(
    app: AppState<'_>,
    payload: RequestOverlayIntent,
) -> IpcResponse<SessionSnapshot> {
    let result = match app.session().report_selection(payload.selection) {
        Ok(()) => Ok(app.session().snapshot()),
        Err(err) => Err(err),
    };
    IpcResponse::from_result(result)
}

/// Commit the current selection. Returns the commit outcome.
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

    let mut rgba = vec![0u8; (bbox.size.width as usize) * (bbox.size.height as usize) * 4];
    for (i, chunk) in rgba.chunks_exact_mut(4).enumerate() {
        let x = (i as u32) % bbox.size.width;
        let y = (i as u32) / bbox.size.width;
        chunk[0] = (x & 0xFF) as u8;
        chunk[1] = (y & 0xFF) as u8;
        chunk[2] = (((x ^ y) >> 1) & 0xFF) as u8;
        chunk[3] = 0xFF;
    }

    let capture_id = uuid::Uuid::new_v4().to_string();
    let path = app.platform().write_png(&capture_id, bbox, &rgba)?;
    let png_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let outcome = CommitOutcome {
        capture_id,
        shelf_id: None,
        png_path: Some(path.to_string_lossy().to_string()),
        png_bytes,
    };

    if let Err(err) = app.session().finish() {
        log::warn!("session.finish failed: {err}");
    }
    Ok(outcome)
}
