//! Thin Tauri command wrappers. The frontend never calls `invoke` directly
//! outside of this module so we can swap to a mock in tests.

import { invoke } from "@tauri-apps/api/core";
import type {
  CacheEntryDto,
  CancelOutcome,
  CaptureResponse,
  CommitResponse,
  CopyShelfCardRequest,
  CopyShelfCardResponse,
  DismissCacheEntryRequest,
  DismissCacheEntryResponse,
  HoverShelfCardRequest,
  IpcResponse,
  RequestCaptureIntent,
  RequestCommitIntent,
  RequestOverlayIntent,
  RequestOverlayResult,
  SaveCaptureAsRequest,
  SaveCaptureAsResponse,
  SaveShelfCardAsRequest,
  SaveShelfCardAsResponse,
  SessionSnapshot,
  ShelfPreferencesDto,
  ShelfQueueSnapshot,
  ShelfSnapshot,
  StartShelfDragIntent,
  StartShelfDragResult,
  UnhoverShelfCardRequest,
  UpdateCacheMetadataRequest,
  UpdateShelfPreferencesRequest,
} from "./types";

export async function requestCapture(
  intent: RequestCaptureIntent,
): Promise<IpcResponse<CaptureResponse>> {
  return invoke<IpcResponse<CaptureResponse>>("request_capture", { payload: intent });
}

export async function requestOverlay(
  payload: RequestOverlayIntent,
): Promise<IpcResponse<RequestOverlayResult>> {
  return invoke<IpcResponse<RequestOverlayResult>>("request_overlay", { payload });
}

export async function requestCommit(
  payload: RequestCommitIntent,
): Promise<IpcResponse<CommitResponse>> {
  return invoke<IpcResponse<CommitResponse>>("request_commit", { payload });
}

export async function requestCancel(): Promise<IpcResponse<CancelOutcome>> {
  return invoke<IpcResponse<CancelOutcome>>("request_cancel");
}

export async function getSessionSnapshot(): Promise<IpcResponse<SessionSnapshot>> {
  return invoke<IpcResponse<SessionSnapshot>>("get_session_snapshot");
}

export async function updateCacheMetadata(
  payload: UpdateCacheMetadataRequest,
): Promise<IpcResponse<CacheEntryDto>> {
  return invoke<IpcResponse<CacheEntryDto>>("update_cache_metadata", { payload });
}

export async function dismissCacheEntry(
  payload: DismissCacheEntryRequest,
): Promise<IpcResponse<DismissCacheEntryResponse>> {
  return invoke<IpcResponse<DismissCacheEntryResponse>>("dismiss_cache_entry", {
    payload,
  });
}

export async function getShelfSnapshot(): Promise<IpcResponse<ShelfSnapshot>> {
  return invoke<IpcResponse<ShelfSnapshot>>("get_shelf_snapshot");
}

export async function getShelfQueueSnapshot(): Promise<IpcResponse<ShelfQueueSnapshot>> {
  return invoke<IpcResponse<ShelfQueueSnapshot>>("get_shelf_queue_snapshot");
}

export async function copyShelfCard(
  payload: CopyShelfCardRequest,
): Promise<IpcResponse<CopyShelfCardResponse>> {
  return invoke<IpcResponse<CopyShelfCardResponse>>("copy_shelf_card", { payload });
}

export async function saveShelfCardAs(
  payload: SaveShelfCardAsRequest,
): Promise<IpcResponse<SaveShelfCardAsResponse>> {
  return invoke<IpcResponse<SaveShelfCardAsResponse>>("save_shelf_card_as", { payload });
}

/// Tracer-05: native Save As for the active session (Ctrl+S). Opens
/// the platform's save dialog and writes the flattened capture (crop
/// + annotations) to the user-chosen path. Returns the chosen path
/// in the success variant; `path = undefined` when the user cancels.
export async function saveCaptureAs(
  payload: SaveCaptureAsRequest,
): Promise<IpcResponse<SaveCaptureAsResponse>> {
  return invoke<IpcResponse<SaveCaptureAsResponse>>("save_capture_as", { payload });
}

export async function hoverShelfCard(
  payload: HoverShelfCardRequest,
): Promise<IpcResponse<ShelfQueueSnapshot>> {
  return invoke<IpcResponse<ShelfQueueSnapshot>>("hover_shelf_card", { payload });
}

export async function unhoverShelfCard(
  payload: UnhoverShelfCardRequest,
): Promise<IpcResponse<ShelfQueueSnapshot>> {
  return invoke<IpcResponse<ShelfQueueSnapshot>>("unhover_shelf_card", { payload });
}

export async function tickShelfQueue(): Promise<IpcResponse<ShelfQueueSnapshot>> {
  return invoke<IpcResponse<ShelfQueueSnapshot>>("tick_shelf_queue");
}

export async function startShelfDrag(
  payload: StartShelfDragIntent,
): Promise<IpcResponse<StartShelfDragResult>> {
  return invoke<IpcResponse<StartShelfDragResult>>("start_shelf_drag", { payload });
}

export async function getShelfPreferences(): Promise<IpcResponse<ShelfPreferencesDto>> {
  return invoke<IpcResponse<ShelfPreferencesDto>>("get_shelf_preferences");
}

export async function updateShelfPreferences(
  payload: UpdateShelfPreferencesRequest,
): Promise<IpcResponse<ShelfPreferencesDto>> {
  return invoke<IpcResponse<ShelfPreferencesDto>>("update_shelf_preferences", { payload });
}
