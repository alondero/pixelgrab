//! Tauri command handlers. Each command is a thin wrapper that translates
//! the WebView payload into the platform capture flow and returns the typed
//! result.

use std::sync::OnceLock;
use std::time::Instant;

use pixelgrab_contracts::{
    cache::CacheEntryMetadata,
    capture::{CaptureFormat, CaptureRequest},
    ipc::{
        CancelOutcome, CaptureResponse, CommitRequest, CommitResponse, DismissCacheEntryRequest,
        DismissCacheEntryResponse, IpcResponse, RequestCaptureIntent, RequestCommitIntent,
        RequestOverlayIntent, RequestOverlayResult, SessionSnapshot, ShelfSnapshot,
        UpdateCacheMetadataRequest,
    },
    shelf_queue::{
        CopyShelfCardRequest, CopyShelfCardResponse, SaveShelfCardAsRequest,
        SaveShelfCardAsResponse, ShelfQueueSnapshot,
    },
    CaptureDiagnostics, PlatformError, PlatformErrorKind, ShelfId,
};
use tauri::{AppHandle, Emitter, State};

use crate::PixelGrabApp;

type AppState<'a> = State<'a, PixelGrabApp>;

/// Monotonic epoch used by the shelf queue timer. Captured lazily on
/// first use so the queue's `added_at_elapsed_ms` and
/// `deadline_at_elapsed_ms` fields are relative to a stable point in
/// the process lifetime rather than the wall-clock — a user
/// NTP-syncing their clock mid-session must not be able to reset a
/// card's countdown.
fn monotonic_epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

/// Current monotonic millis since process start (relative to
/// [`monotonic_epoch`]). The shelf queue engine treats this as the
/// authoritative "now" so timers cannot be defeated by clock changes.
pub fn now_ms() -> i64 {
    monotonic_epoch().elapsed().as_millis() as i64
}

/// Resolve the shelf position for the queue's current visible card
/// count. Returns `None` when the platform cannot enumerate monitors
/// (callers should hide the shelf window in that case).
fn queue_position(app: &PixelGrabApp) -> Result<pixelgrab_contracts::ShelfPosition, PlatformError> {
    let layout = app.platform().monitor_layout()?;
    let monitor = layout
        .monitors
        .iter()
        .find(|m| m.is_primary)
        .or_else(|| layout.monitors.first())
        .ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::MonitorQueryFailed,
                "no monitor available for shelf placement",
            )
        })?;
    let visible = app.shelf_queue().snapshot(now_ms()).cards.len();
    Ok(pixelgrab_contracts::ShelfPosition::shelf_queue_position(
        monitor, visible,
    ))
}

/// Emit the shelf-queue-updated event with the latest snapshot. The
/// frontend listener is idempotent and re-renders the queue from the
/// payload alone.
fn emit_shelf_queue_updated<R: tauri::Runtime>(
    handle: &AppHandle<R>,
    snapshot: ShelfQueueSnapshot,
) {
    let _ = handle.emit("pixelgrab://shelf-queue-updated", &snapshot);
}

/// Build a snapshot with the position field populated. Used by every
/// event-emit path so the frontend never has to compute window
/// geometry itself. The mutation-only `with_position` variant is
/// kept as a thin alias for the few call sites that already hold a
/// snapshot they want to decorate.
fn snapshot_with_position(app: &PixelGrabApp) -> ShelfQueueSnapshot {
    with_position(app.shelf_queue().snapshot(now_ms()), app)
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

/// Dismiss a shelf card. Removes the card from the queue, releases
/// the `Shelf` lock, and reaps the entry from disk when no other
/// owners hold it. The shelf window hides itself when the queue is
/// empty afterwards.
#[tauri::command]
pub fn dismiss_cache_entry(
    app: AppState<'_>,
    payload: DismissCacheEntryRequest,
    handle: AppHandle,
) -> IpcResponse<DismissCacheEntryResponse> {
    // Dismiss ordering matters: the spec requires the shelf lock to
    // remain held until the card has left every shelf representation
    // (main view + overflow). Remove the card from the queue first,
    // then dismiss from the cache so the lock release coincides with
    // (rather than precedes) the user-visible disappearance.
    app.shelf_queue().dismiss(&payload.shelf_id, now_ms());
    let result = match app.cache().dismiss(&payload.shelf_id) {
        Ok(outcome) => {
            // When the cache reaped the entry we also emit a cleared
            // event so listeners that don't care about queue ordering
            // still learn about the removal.
            let snapshot = snapshot_with_position(&app);
            if snapshot.is_empty() {
                let _ = crate::shelf::hide_card(&handle);
            }
            emit_shelf_queue_updated(&handle, snapshot);
            if outcome.removed {
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

/// Copy a shelf card's flattened PNG to the system clipboard. Reads
/// the PNG bytes from the cache root and forwards them to the
/// platform's native clipboard.
#[tauri::command]
pub fn copy_shelf_card(
    app: AppState<'_>,
    payload: CopyShelfCardRequest,
) -> IpcResponse<CopyShelfCardResponse> {
    let png_path = match app.shelf_queue().png_path(&payload.shelf_id) {
        Some(p) => p,
        None => {
            return IpcResponse::from_result(Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("unknown shelf id: {}", payload.shelf_id),
            )));
        }
    };
    let bytes = match app
        .platform()
        .publish_png_clipboard(std::path::Path::new(&png_path))
    {
        Ok(()) => std::fs::metadata(&png_path).map(|m| m.len()).unwrap_or(0),
        Err(err) => return IpcResponse::from_result(Err(err)),
    };
    IpcResponse::from_result(Ok(CopyShelfCardResponse { png_bytes: bytes }))
}

/// Save a shelf card's PNG to a user-chosen location via the native
/// Save As dialog. Returns `path = None` when the user cancels. The
/// blocking dialog call is dispatched onto a worker thread so the
/// async command future is `'static`-friendly. Tauri's async command
/// macro requires a `Result` return when the inputs contain
/// references, so the inner `IpcResponse` is wrapped in `Ok` here and
/// surfaced as the Ok variant.
#[tauri::command]
pub async fn save_shelf_card_as(
    app: AppState<'_>,
    payload: SaveShelfCardAsRequest,
    handle: AppHandle,
) -> Result<IpcResponse<SaveShelfCardAsResponse>, PlatformError> {
    use tauri_plugin_dialog::DialogExt;
    let png_path = match app.shelf_queue().png_path(&payload.shelf_id) {
        Some(p) => p,
        None => {
            return Ok(IpcResponse::from_result(Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!("unknown shelf id: {}", payload.shelf_id),
            ))));
        }
    };
    let bytes = match std::fs::read(&png_path) {
        Ok(b) => b,
        Err(_err) => {
            // Privacy: do not interpolate the io::Error into the
            // user-facing message — on Windows its Display impl can
            // include the absolute path that failed. Use a stable
            // categorical kind instead; the cache holds the only
            // copy of the path.
            return Ok(IpcResponse::from_result(Err(PlatformError::new(
                PlatformErrorKind::Io,
                "save_as_read_failed",
            ))));
        }
    };
    let suggested = std::path::Path::new(&png_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("capture.png")
        .to_string();
    let chosen = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .add_filter("PNG image", &["png"])
            .set_file_name(&suggested)
            .blocking_save_file()
    })
    .await
    .unwrap_or(None);
    let Some(target) = chosen else {
        return Ok(IpcResponse::from_result(Ok(SaveShelfCardAsResponse {
            path: None,
            png_bytes: 0,
        })));
    };
    let target_path = match target.into_path() {
        Ok(p) => p,
        Err(_err) => {
            return Ok(IpcResponse::from_result(Err(PlatformError::new(
                PlatformErrorKind::Io,
                "save_as_invalid_target",
            ))));
        }
    };
    if let Err(_err) = std::fs::write(&target_path, &bytes) {
        // Privacy: same as above — categorical kind, no path in the
        // error string. The chosen path itself is returned in the
        // Ok variant only, so the user can still see where the file
        // landed when the write succeeds.
        return Ok(IpcResponse::from_result(Err(PlatformError::new(
            PlatformErrorKind::Io,
            "save_as_write_failed",
        ))));
    }
    let written = bytes.len() as u64;
    Ok(IpcResponse::from_result(Ok(SaveShelfCardAsResponse {
        path: Some(target_path.to_string_lossy().to_string()),
        png_bytes: written,
    })))
}

/// Mark a shelf card as hovered at the current monotonic millis.
/// Only the targeted card's timer pauses.
#[tauri::command]
pub fn hover_shelf_card(
    app: AppState<'_>,
    payload: HoverShelfCardRequest,
    handle: AppHandle,
) -> IpcResponse<ShelfQueueSnapshot> {
    let result = match app.shelf_queue().hover(&payload.shelf_id, now_ms()) {
        Some(snapshot) => {
            let snapshot = with_position(snapshot, &app);
            emit_shelf_queue_updated(&handle, snapshot.clone());
            Ok(snapshot)
        }
        None => Err(PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!("unknown shelf id: {}", payload.shelf_id),
        )),
    };
    IpcResponse::from_result(result)
}

/// Mark a shelf card as un-hovered at the current monotonic millis.
/// Resumes the timer with a three-second grace period when the
/// remaining time is small.
#[tauri::command]
pub fn unhover_shelf_card(
    app: AppState<'_>,
    payload: UnhoverShelfCardRequest,
    handle: AppHandle,
) -> IpcResponse<ShelfQueueSnapshot> {
    let result = match app.shelf_queue().unhover(&payload.shelf_id, now_ms()) {
        Some(snapshot) => {
            let snapshot = with_position(snapshot, &app);
            emit_shelf_queue_updated(&handle, snapshot.clone());
            Ok(snapshot)
        }
        None => Err(PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!("unknown shelf id: {}", payload.shelf_id),
        )),
    };
    IpcResponse::from_result(result)
}

/// Tick the queue. Removes expired cards and dismisses each one from
/// the cache so the shelf lock is released and the entry reaped.
/// Triggered by the frontend after a countdown animation reaches zero,
/// and periodically by the background ticker installed in
/// `PixelGrabApp::install_shelf_ticker` so a hidden or throttled
/// webview cannot strand the shelf lock.
#[tauri::command]
pub fn tick_shelf_queue(app: AppState<'_>) -> IpcResponse<ShelfQueueSnapshot> {
    let outcome = app.shelf_queue().tick(now_ms());
    for shelf_id in &outcome.expired {
        // Privacy: only the shelf id is logged. The cache dismiss
        // error can include the cache path; a stable categorical
        // description is sufficient for telemetry.
        if let Err(_err) = app.cache().dismiss(shelf_id) {
            log::warn!("tick_shelf_queue: cache.dismiss failed for shelf_id");
        }
    }
    let snapshot = with_position(outcome.snapshot, &app);
    IpcResponse::from_result(Ok(snapshot))
}

/// Wire payload for the `pixelgrab://shelf-cleared` event.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShelfClearedEvent {
    /// Shelf id that was dismissed.
    shelf_id: String,
}

/// Wire payload for the `hover_shelf_card` IPC.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverShelfCardRequest {
    /// Shelf card id to mark as hovered.
    pub shelf_id: ShelfId,
}

/// Wire payload for the `unhover_shelf_card` IPC.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnhoverShelfCardRequest {
    /// Shelf card id to mark as un-hovered.
    pub shelf_id: ShelfId,
}

/// Fill in the position field on a freshly-built snapshot. Pulled out
/// so the hover / unhover / tick handlers can stay focused on their
/// event semantics.
fn with_position(mut snapshot: ShelfQueueSnapshot, app: &PixelGrabApp) -> ShelfQueueSnapshot {
    if let Ok(position) = queue_position(app) {
        snapshot.position = Some(position);
    }
    snapshot
}

/// Snapshot of the current shelf queue. Used by the frontend to
/// rehydrate the queue UI on startup. Introduced by tracer 08.
#[tauri::command]
pub fn get_shelf_queue_snapshot(app: AppState<'_>) -> IpcResponse<ShelfQueueSnapshot> {
    IpcResponse::from_result(Ok(snapshot_with_position(&app)))
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

                    // Push the new card onto the queue and emit a
                    // queue snapshot. The shelf window is shown with
                    // the new multi-card geometry.
                    app.shelf_queue().add(entry.clone(), now_ms());
                    let snapshot = snapshot_with_position(app);
                    if let Some(position) = snapshot.position.as_ref() {
                        if let Err(err) = crate::shelf::show_queue(handle, position) {
                            log::warn!("shelf window show failed: {err}");
                        }
                    }
                    emit_shelf_queue_updated(handle, snapshot);
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
