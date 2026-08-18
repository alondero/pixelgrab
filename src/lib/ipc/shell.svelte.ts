// In-browser mock for the Tauri command surface. Used by Vitest so component
// tests can exercise the contract without the Tauri runtime. The mock
// reproduces the deterministic synthetic capture described in ADR-0004.

import type {
  CaptureResolutionDto,
  CommitResponse,
  IpcResponse,
  PhysicalBounds,
  RequestCaptureIntent,
  RequestCommitIntent,
  RequestOverlayIntent,
  SessionSnapshot,
  SessionState,
} from "./types";

const sessionState: { value: SessionState } = $state({ value: "idle" });
let lastCapture: CaptureResolutionDto | undefined;
let selection: PhysicalBounds | undefined;

/** Read the current session state for tests. */
export function mockSessionState(): SessionState {
  return sessionState.value;
}

function ok<T>(data: T): IpcResponse<T> {
  return { status: "ok", data };
}

function err<T>(message: string): IpcResponse<T> {
  return {
    status: "err",
    error: { kind: "internal", message },
  };
}

export async function mockRequestCapture(
  intent: RequestCaptureIntent,
): Promise<IpcResponse<CaptureResolutionDto>> {
  if (sessionState.value !== "idle") {
    return err("capture already in progress");
  }
  sessionState.value = "capturing";
  const captureId = crypto.randomUUID();
  const capture: CaptureResolutionDto = {
    format: "virtual_desktop",
    bounds: { origin: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } },
    assetUrl: `data:image/png;base64,${syntheticPngBase64(intent.intent)}`,
    captureId,
    capturedAtMs: Date.now(),
  };
  lastCapture = capture;
  sessionState.value = "ready";
  return ok(capture);
}

export async function mockRequestOverlay(
  payload: RequestOverlayIntent,
): Promise<IpcResponse<SessionSnapshot>> {
  if (sessionState.value !== "ready" && sessionState.value !== "selecting") {
    return err("overlay not ready");
  }
  selection = payload.selection;
  sessionState.value = "selecting";
  return ok({ state: sessionState.value, lastCapture, selection });
}

export async function mockRequestCommit(
  payload: RequestCommitIntent,
): Promise<IpcResponse<CommitResponse>> {
  if (sessionState.value !== "selecting") {
    return err("nothing to commit");
  }
  const captureId = crypto.randomUUID();
  sessionState.value = "cleanup";
  sessionState.value = "idle";
  selection = undefined;
  return ok({
    outcome: {
      captureId,
      pngBytes: payload.crop.size.width * payload.crop.size.height * 4,
      pngPath: `/tmp/${captureId}.png`,
    },
  });
}

export async function mockGetSessionSnapshot(): Promise<IpcResponse<SessionSnapshot>> {
  return ok({ state: sessionState.value, lastCapture, selection });
}

export function __resetMock() {
  sessionState.value = "idle";
  lastCapture = undefined;
  selection = undefined;
}

/**
 * Encode a deterministic 1x1 PNG in base64 so the mock capture has a real
 * asset URL pointing at bytes.
 */
function syntheticPngBase64(_intent: RequestCaptureIntent["intent"]): string {
  // Pre-encoded 1x1 transparent PNG.
  return "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
}
