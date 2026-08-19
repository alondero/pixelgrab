//! Thin Tauri command wrappers. The frontend never calls `invoke` directly
//! outside of this module so we can swap to a mock in tests.

import { invoke } from "@tauri-apps/api/core";
import type {
  CacheEntryDto,
  CancelOutcome,
  CaptureResponse,
  CommitResponse,
  DismissCacheEntryRequest,
  DismissCacheEntryResponse,
  IpcResponse,
  RequestCaptureIntent,
  RequestCommitIntent,
  RequestOverlayIntent,
  RequestOverlayResult,
  SessionSnapshot,
  ShelfSnapshot,
  UpdateCacheMetadataRequest,
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
