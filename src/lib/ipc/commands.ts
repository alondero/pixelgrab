// Thin Tauri command wrappers. The frontend never calls `invoke` directly
// outside of this module so we can swap to a mock in tests.

import { invoke } from "@tauri-apps/api/core";
import type {
  CancelOutcome,
  CaptureResponse,
  CommitResponse,
  IpcResponse,
  RequestCaptureIntent,
  RequestCommitIntent,
  RequestOverlayIntent,
  RequestOverlayResult,
  SessionSnapshot,
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
