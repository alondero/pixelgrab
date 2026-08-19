//! Tauri command handlers. Each command is a thin wrapper that translates
//! the WebView payload into the platform capture flow and returns the typed
//! result.

use std::time::{SystemTime, UNIX_EPOCH};

use pixelgrab_contracts::{
    cache::CacheEntryMetadata,
    capture::{CaptureFormat, CaptureRequest},
    ipc::{
        CancelOutcome, CaptureResponse, CommitRequest, CommitResponse, DismissCacheEntryRequest,
        DismissCacheEntryResponse, IpcResponse, RequestCaptureIntent, RequestCommitIntent,
        RequestOverlayIntent, RequestOverlayResult, SessionSnapshot, ShelfSnapshot,
        UpdateCacheMetadataRequest,
    },
    CaptureDiagnostics, PlatformError, PlatformErrorKind,
};
use tauri::{AppHandle, Emitter, State};

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

/// Cancel the active session. Honours the staged Escape behaviour.
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
/// crop is the single source from which both the on-disk PNG (via the
/// cache store's two-phase commit) and the clipboard bitmap
/// representation are derived.
///
/// `to_shelf` and `to_clipboard` both default to true so the press of
/// Enter covers the full tracer-07 commitment (atomic cache + clipboard +
/// shelf card). When either is false the corresponding effect is skipped
/// (e.g. the tray's "copy only" command).
#[tauri::command]
pub fn request_commit(
    app: AppState<'_>,
    payload: RequestCommitIntent,
    handle: AppHandle,
) -> IpcResponse<CommitResponse> {
    let commit_request = CommitRequest {
        crop: payload.crop,
        to_shelf: payload.to_shelf,
        to_clipboard: payload.to_clipboard,
        save_as: payload.save_as,
    };
    let result = match commit(&app, &handle, &commit_request) {
        Ok(outcome) => Ok(CommitResponse { outcome }),
        Err(err) => Err(err),
    };
    IpcResponse::from_result(result)
}

/// Update the editable metadata for a shelf card. The cache rewrites
/// `metadata.json` atomically and refreshes the manifest's
/// `last_access_at_ms`.
#[tauri::command]
pub fn update_cache_metadata(
    app: AppState<'_>,
    payload: UpdateCacheMetadataRequest,
    handle: AppHandle,
) -> IpcResponse<pixelgrab_contracts::CacheEntry> {
    let result = app
        .cache()
        .update_metadata(&payload.shelf_id, payload.metadata.clone());
    if result.is_ok() {
        if let Ok(entry) = &result {
            emit_shelf_updated(&handle, entry);
        }
    }
    IpcResponse::from_result(result)
}

/// Dismiss a shelf card. Releases the `Shelf` lock and reaps the entry
/// from disk when no other owners hold it.
#[tauri::command]
pub fn dismiss_cache_entry(
    app: AppState<'_>,
    payload: DismissCacheEntryRequest,
    handle: AppHandle,
) -> IpcResponse<DismissCacheEntryResponse> {
    let result = match app.cache().dismiss(&payload.shelf_id) {
        Ok(outcome) => {
            // Hide the shelf window when the only card is gone and
            // tell the frontend to clear its local copy. The event
            // payload is the dismissed shelf id, wrapped in a
            // typed struct so the TS listener can match its
            // parameter without `any`.
            if outcome.removed {
                let _ = crate::shelf::hide_card(&handle);
                let event = ShelfClearedEvent {
                    shelf_id: payload.shelf_id.clone(),
                };
                let _ = handle.emit("pixelgrab://shelf-cleared", &event);
            }
            Ok(DismissCacheEntryResponse {
                removed: outcome.removed,
                reason: outcome.reason.to_string(),
            })
        }
        Err(err) => Err(err),
    };
    IpcResponse::from_result(result)
}

/// Wire payload for the `pixelgrab://shelf-cleared` event.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShelfClearedEvent {
    /// Shelf id that was dismissed.
    shelf_id: String,
}

/// Snapshot of the current shelf state. Used by the frontend to
/// rehydrate the shelf UI on startup.
#[tauri::command]
pub fn get_shelf_snapshot(app: AppState<'_>) -> IpcResponse<ShelfSnapshot> {
    let entries = app.cache().entries();
    let layout = app.platform().monitor_layout();
    let locks = app.cache().locks();
    let snapshot = match layout {
        Ok(layout) => {
            let entry = entries.last().cloned();
            let position = entry
                .as_ref()
                .and_then(|e| app.cache().shelf_position(&e.shelf_id, &layout).ok());
            let lock_owners = match &entry {
                Some(e) => locks.owners_of(&e.shelf_id),
                None => Vec::new(),
            };
            ShelfSnapshot {
                entry,
                position,
                locks: lock_owners,
            }
        }
        Err(err) => {
            return IpcResponse::from_result(Err(err));
        }
    };
    IpcResponse::from_result(Ok(snapshot))
}

/// Read the current session snapshot.
#[tauri::command]
pub fn get_session_snapshot(app: AppState<'_>) -> IpcResponse<SessionSnapshot> {
    IpcResponse::from_result(Ok(app.session().snapshot()))
}

/// Internal helper: emit a shelf-updated event to the frontend.
/// The shelf id is part of the view so the trait doesn't need it as
/// a separate parameter.
fn emit_shelf_updated<R: tauri::Runtime>(
    handle: &AppHandle<R>,
    entry: &pixelgrab_contracts::CacheEntry,
) {
    let view = crate::shelf::ShelfCardView::from_entry(entry);
    let _ = handle.emit("pixelgrab://shelf-updated", &view);
}

fn commit(
    app: &PixelGrabApp,
    handle: &AppHandle,
    request: &CommitRequest,
) -> Result<pixelgrab_contracts::ipc::CommitOutcome, PlatformError> {
    use pixelgrab_contracts::ipc::CommitOutcome;
    let bbox = request.crop;
    bbox.validate().map_err(|_| {
        PlatformError::new(PlatformErrorKind::InvalidPayload, "empty commit bounds")
    })?;

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

    // Single source of truth: flatten the crop once. The PNG bytes and
    // the clipboard bitmap are both derived from this buffer. Validate
    // the buffer length here so a corrupt platform response never
    // reaches either the clipboard or the cache; the cache's
    // `encode_png` re-validates as the last line of defence.
    let (rgba, size) = app.platform().flatten_crop(&capture_id, bbox)?;
    let expected_len = (size.width as usize) * (size.height as usize) * 4;
    if rgba.len() != expected_len {
        return Err(PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!(
                "flatten_crop returned {} bytes for {}x{}; expected {}",
                rgba.len(),
                size.width,
                size.height,
                expected_len
            ),
        ));
    }

    let mut outcome = CommitOutcome {
        capture_id: capture_id.clone(),
        shelf_id: None,
        png_path: None,
        png_bytes: 0,
        size_bytes: 0,
        created_at_ms: 0,
    };

    // The commit body collects every side effect into `commit_result`
    // so the function can run `session.finish()` exactly once before
    // returning — even on clipboard or cache failures. Otherwise the
    // session would stay in `Committing` and block the next capture.
    let commit_result: Result<(), PlatformError> = (|| {
        // Clipboard first. The clipboard publish is the cheapest
        // non-reversible side effect; if it fails we abort before
        // touching the cache or the shelf, so a clipboard error never
        // leaves a phantom card.
        if request.to_clipboard {
            app.platform().publish_clipboard(&capture_id, &rgba, size)?;
        }

        if request.to_shelf {
            let primary_monitor_id =
                crate::cache::Cache::primary_monitor_id(&app.platform().monitor_layout()?)?;
            let commit = app.cache().commit(crate::cache::CacheCommitRequest {
                bounds: bbox,
                size,
                rgba: rgba.clone(),
                metadata: CacheEntryMetadata::default(),
                monitor_id: primary_monitor_id.clone(),
            });
            match commit {
                Ok(commit_result) => {
                    let entry = commit_result.entry;
                    outcome.shelf_id = Some(entry.shelf_id.clone());
                    outcome.png_path = Some(entry.png_path.clone());
                    outcome.png_bytes = commit_result.png_bytes;
                    outcome.size_bytes = entry.size_bytes;
                    outcome.created_at_ms = entry.created_at_ms;

                    // Compute the shelf placement and show the card.
                    // The cache owns the shelf lock until the entry
                    // is dismissed, so we don't need to keep a guard
                    // alive here.
                    let layout = app.platform().monitor_layout()?;
                    let position = app.cache().shelf_position(&entry.shelf_id, &layout)?;
                    if let Err(err) = crate::shelf::show_card(handle, &position, &entry.shelf_id) {
                        log::warn!("shelf window show failed: {err}");
                    }
                    emit_shelf_updated(handle, &entry);
                }
                Err(err) => {
                    // Two-phase commit failed: the entry is either
                    // fully absent or partial. Surface the error and
                    // leave the shelf empty.
                    log::warn!("cache commit failed: {err}");
                    return Err(err);
                }
            }
        }

        if request.save_as {
            // Save-as writes a loose PNG to the platform's cache
            // root in addition to whatever the shelf path produced.
            // `to_shelf` and `save_as` are independent; a commit
            // with both flags shelves the capture AND produces a
            // loose copy. The `if outcome.png_path.is_none()` guard
            // keeps the shelf path's png_path authoritative when
            // both flags are set.
            let path = app.platform().write_png(&capture_id, bbox, &rgba)?;
            if outcome.png_path.is_none() {
                outcome.png_path = Some(path.to_string_lossy().to_string());
                let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                outcome.png_bytes = bytes;
            }
        }

        if !request.to_shelf && !request.save_as {
            outcome.png_bytes = rgba.len() as u64;
        }

        Ok(())
    })();

    if let Err(err) = app.session().finish() {
        log::warn!("session.finish failed: {err}");
    }
    commit_result.map(|_| outcome)
}

/// Derive a diagnostic monitor id from a capture resolution.
fn monitor_id_for(capture: &pixelgrab_contracts::capture::CaptureResolution) -> String {
    use pixelgrab_contracts::capture::CaptureFormat;
    match capture.format {
        CaptureFormat::VirtualDesktop => "virtual-desktop".into(),
        CaptureFormat::SingleMonitor => "single-monitor".into(),
        CaptureFormat::PhysicalRegion => "physical-region".into(),
    }
}
