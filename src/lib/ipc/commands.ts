//! Thin Tauri command wrappers. The frontend never calls `invoke` directly
//! outside of this module so we can swap to a mock in tests.

import { invoke } from "@tauri-apps/api/core";
import type {
  CacheEntryDto,
  CancelOutcome,
  CancelRevisionIntent,
  CancelRevisionResult,
  CaptureResponse,
  CommitResponse,
  CommitRevisionIntent,
  CommitRevisionResult,
  CopyShelfCardRequest,
  CopyShelfCardResponse,
  DismissCacheEntryRequest,
  DismissCacheEntryResponse,
  HotkeyBindingsDto,
  HotkeyRegistryStatusDto,
  HoverShelfCardRequest,
  IpcResponse,
  OpenRevisionIntent,
  OpenRevisionResult,
  RequestCaptureIntent,
  RequestCommitIntent,
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
  UpdateHotkeyBindingsRequest,
  UpdateRevisionIntent,
  UpdateRevisionResult,
  UpdateShelfPreferencesRequest,
} from "./types";

export async function requestCapture(
  intent: RequestCaptureIntent,
): Promise<IpcResponse<CaptureResponse>> {
  return invoke<IpcResponse<CaptureResponse>>("request_capture", { payload: intent });
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

/// Show and focus the main companion window. Used by the shelf
/// webview after reopening a card for editing so the revision editor
/// becomes visible.
export async function showMainWindow(): Promise<IpcResponse<null>> {
  return invoke<IpcResponse<null>>("show_main_window");
}

export async function getShelfPreferences(): Promise<IpcResponse<ShelfPreferencesDto>> {
  return invoke<IpcResponse<ShelfPreferencesDto>>("get_shelf_preferences");
}

export async function updateShelfPreferences(
  payload: UpdateShelfPreferencesRequest,
): Promise<IpcResponse<ShelfPreferencesDto>> {
  return invoke<IpcResponse<ShelfPreferencesDto>>("update_shelf_preferences", { payload });
}

// ---------------------------------------------------------------------------
// Hotkey bindings (tracer 14). The frontend owns the typed wrapper; the
// Rust core owns the canonical state and re-applies every bulk update.
// ---------------------------------------------------------------------------

export async function getHotkeyBindings(): Promise<IpcResponse<HotkeyBindingsDto>> {
  return invoke<IpcResponse<HotkeyBindingsDto>>("get_hotkey_bindings");
}

export async function updateHotkeyBindings(
  payload: UpdateHotkeyBindingsRequest,
): Promise<IpcResponse<HotkeyBindingsDto>> {
  return invoke<IpcResponse<HotkeyBindingsDto>>("update_hotkey_bindings", { payload });
}

export async function getHotkeyStatus(): Promise<IpcResponse<HotkeyRegistryStatusDto>> {
  return invoke<IpcResponse<HotkeyRegistryStatusDto>>("get_hotkey_status");
}

export async function setHotkeyPaused(
  paused: boolean,
): Promise<IpcResponse<HotkeyRegistryStatusDto>> {
  return invoke<IpcResponse<HotkeyRegistryStatusDto>>("set_hotkey_paused", { paused });
}

// ---------------------------------------------------------------------------
// Tracer-10: reopen / non-destructive revision IPC.
// ---------------------------------------------------------------------------

/// Open a shelf entry for non-destructive editing. Acquires the
/// `Editor` lock on the source entry, reads the `revision.json`
/// sidecar (or falls back to the flat PNG when the sidecar is
/// missing / unparseable / has an unsupported version), and
/// returns the restored editor scene.
export async function openRevision(
  payload: OpenRevisionIntent,
): Promise<IpcResponse<OpenRevisionResult>> {
  return invoke<IpcResponse<OpenRevisionResult>>("open_revision", { payload });
}

/// Persist the in-progress editor scene to the source entry's
/// `revision.json` without committing. The frontend drives this
/// from a debounced handler on every annotation change.
export async function updateRevision(
  payload: UpdateRevisionIntent,
): Promise<IpcResponse<UpdateRevisionResult>> {
  return invoke<IpcResponse<UpdateRevisionResult>>("update_revision", { payload });
}

/// Commit the editor scene as a revised capture. The source
/// entry's assets remain untouched; the new entry has a distinct
/// `captureId` and `shelfId`.
export async function commitRevision(
  payload: CommitRevisionIntent,
): Promise<IpcResponse<CommitRevisionResult>> {
  return invoke<IpcResponse<CommitRevisionResult>>("commit_revision", { payload });
}

/// Cancel a reopen session. Releases the editor lock on the
/// source entry and resets the session to `Idle`.
export async function cancelRevision(
  payload: CancelRevisionIntent,
): Promise<IpcResponse<CancelRevisionResult>> {
  return invoke<IpcResponse<CancelRevisionResult>>("cancel_revision", { payload });
}
