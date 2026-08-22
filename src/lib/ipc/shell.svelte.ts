// In-browser mock for the Tauri command surface. Used by Vitest so component
// tests can exercise the contract without the Tauri runtime. The mock
// reproduces the deterministic synthetic capture described in ADR-0004.

import type {
  CancelOutcome,
  CaptureDiagnostics,
  CaptureResponse,
  CommitResponse,
  HotkeyBindingsDto,
  HotkeyRegistryStatusDto,
  IpcResponse,
  PhysicalBounds,
  RequestCaptureIntent,
  RequestCommitIntent,
  SessionSnapshot,
  SessionState,
  ShelfPreferencesDto,
  StartShelfDragIntent,
  StartShelfDragResult,
  UpdateHotkeyBindingsRequest,
  UpdateShelfPreferencesRequest,
} from "./types";

const sessionState: { value: SessionState } = $state({ value: "idle" });
let lastCapture:
  | {
      captureId: string;
      bounds: PhysicalBounds;
    }
  | undefined;
let lastDiagnostics: CaptureDiagnostics | undefined;
let selection: PhysicalBounds | undefined;
// Tracer 12: in-memory mock for shelf preferences so the App can
// render the SettingsPanel without the Tauri runtime. Updated by
// `mockUpdateShelfPreferences`, read by `mockGetShelfPreferences`.
let preferences: ShelfPreferencesDto = {
  schemaVersion: 1,
  corner: "bottom_right",
  targetMonitorId: null,
  marginPx: 24,
  autoDismissEnabled: true,
  lifetimeSeconds: 60,
  visibleCardCount: 4,
  showCountdown: true,
};

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
): Promise<IpcResponse<CaptureResponse>> {
  if (sessionState.value !== "idle") {
    return err("capture already in progress");
  }
  sessionState.value = "capturing";
  const startedAt = Date.now();
  const captureId = crypto.randomUUID();
  const bounds = { origin: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } };
  const capture = {
    format: "virtual_desktop" as const,
    bounds,
    assetUrl: `data:image/png;base64,${syntheticPngBase64(intent.intent)}`,
    captureId,
    capturedAtMs: Date.now(),
  };
  lastCapture = { captureId, bounds };
  lastDiagnostics = {
    captureId,
    captureStartedAtMs: startedAt,
    captureCompletedAtMs: Date.now(),
    captureDurationMs: Date.now() - startedAt,
    monitorId: "virtual-desktop",
    bounds,
  };
  // Issue #60: the overlay reveal contract is collapsed into one backend
  // seam. The mock mirrors that shape — a successful capture walks the
  // session from `Idle` to `Selecting` in a single call, so the frontend
  // never has to drive a separate overlay IPC to land on `Selecting`.
  sessionState.value = "selecting";
  if (lastDiagnostics) {
    lastDiagnostics = {
      ...lastDiagnostics,
      overlayVisibleAtMs: Date.now(),
      captureToOverlayMs: Date.now() - lastDiagnostics.captureCompletedAtMs,
    };
  }
  return ok({ capture, diagnostics: lastDiagnostics });
}

export async function mockRequestCommit(
  payload: RequestCommitIntent,
): Promise<IpcResponse<CommitResponse>> {
  if (sessionState.value !== "selecting") {
    return err("nothing to commit");
  }
  const captureId = crypto.randomUUID();
  sessionState.value = "committing";
  sessionState.value = "cleanup";
  selection = undefined;
  lastDiagnostics = undefined;
  lastCapture = undefined;
  sessionState.value = "idle";
  return ok({
    outcome: {
      captureId,
      pngBytes: payload.crop.size.width * payload.crop.size.height * 4,
      pngPath: `/tmp/${captureId}.png`,
    },
  });
}

export async function mockRequestCancel(): Promise<IpcResponse<CancelOutcome>> {
  const state = sessionState.value;
  let action: CancelOutcome["action"] = "noop";
  if (state === "selecting" && hasSelection()) {
    selection = undefined;
    action = "selection_cleared";
  } else if (state !== "idle") {
    action = "session_cancelled";
    sessionState.value = "cleanup";
    sessionState.value = "idle";
    selection = undefined;
    lastDiagnostics = undefined;
    lastCapture = undefined;
  }
  return ok({
    action,
    snapshot: {
      state: sessionState.value,
      selection,
    },
  });
}

function hasSelection(): boolean {
  if (!selection) return false;
  return selection.size.width > 0 && selection.size.height > 0;
}

export async function mockGetSessionSnapshot(): Promise<IpcResponse<SessionSnapshot>> {
  return ok({
    state: sessionState.value,
    lastCapture: lastCapture
      ? {
          format: "virtual_desktop",
          bounds: lastCapture.bounds,
          assetUrl: `data:image/png;base64,${syntheticPngBase64("region")}`,
          captureId: lastCapture.captureId,
          capturedAtMs: Date.now(),
        }
      : undefined,
    selection,
  });
}

export async function mockStartShelfDrag(
  payload: StartShelfDragIntent,
): Promise<IpcResponse<StartShelfDragResult>> {
  const startedAt = Date.now();
  // The mock always returns "cancelled" — the same outcome the
  // synthetic adapter returns when the script is `Stable`. The shelf
  // card remains visible so the user can retry.
  const completedAt = Date.now();
  return ok({
    outcome: "cancelled",
    diagnostics: {
      startedAtMs: startedAt,
      completedAtMs: completedAt,
      durationMs: completedAt - startedAt,
      targetEffect: "unknown",
      targetKind: "none",
      captureId: "mock-capture",
      shelfId: payload.shelfId,
    },
    shouldDismiss: false,
  });
}

// Tracer 14 mocks: in-memory hotkey bindings so the HotkeyPanel
// can render and update without a Tauri runtime. Mirrors the
// Rust-side defaults so the reset path is exercised end-to-end.

let hotkeyBindings: HotkeyBindingsDto = {
  schemaVersion: 1,
  regionCapture: "CommandOrControl+Shift+S",
  fullScreenCapture: "CommandOrControl+Shift+F",
  shelfToggle: "CommandOrControl+Shift+L",
  paused: false,
};
let hotkeyStatus: HotkeyRegistryStatusDto = {
  active: true,
  paused: false,
};

export async function mockGetHotkeyBindings(): Promise<IpcResponse<HotkeyBindingsDto>> {
  return ok({ ...hotkeyBindings });
}

export async function mockUpdateHotkeyBindings(
  payload: UpdateHotkeyBindingsRequest,
): Promise<IpcResponse<HotkeyBindingsDto>> {
  hotkeyBindings = { ...hotkeyBindings, ...payload.bindings };
  hotkeyStatus = {
    ...hotkeyStatus,
    paused: Boolean(hotkeyBindings.paused),
    active: !hotkeyBindings.paused,
  };
  return ok({ ...hotkeyBindings });
}

export async function mockGetHotkeyStatus(): Promise<IpcResponse<HotkeyRegistryStatusDto>> {
  return ok({ ...hotkeyStatus });
}

export async function mockSetHotkeyPaused(
  paused: boolean,
): Promise<IpcResponse<HotkeyRegistryStatusDto>> {
  hotkeyBindings = { ...hotkeyBindings, paused };
  hotkeyStatus = { ...hotkeyStatus, paused, active: !paused };
  return ok({ ...hotkeyStatus });
}

export async function mockGetShelfPreferences(): Promise<IpcResponse<ShelfPreferencesDto>> {
  return ok({ ...preferences });
}

export async function mockUpdateShelfPreferences(
  payload: UpdateShelfPreferencesRequest,
): Promise<IpcResponse<ShelfPreferencesDto>> {
  preferences = { ...preferences, ...payload.preferences };
  return ok({ ...preferences });
}

export function __resetMock() {
  sessionState.value = "idle";
  lastCapture = undefined;
  lastDiagnostics = undefined;
  selection = undefined;
  preferences = {
    schemaVersion: 1,
    corner: "bottom_right",
    targetMonitorId: null,
    marginPx: 24,
    autoDismissEnabled: true,
    lifetimeSeconds: 60,
    visibleCardCount: 4,
    showCountdown: true,
  };
  hotkeyBindings = {
    schemaVersion: 1,
    regionCapture: "CommandOrControl+Shift+S",
    fullScreenCapture: "CommandOrControl+Shift+F",
    shelfToggle: "CommandOrControl+Shift+L",
    paused: false,
  };
  hotkeyStatus = { active: true, paused: false };
}

/**
 * Encode a deterministic 1x1 PNG in base64 so the mock capture has a real
 * asset URL pointing at bytes.
 */
function syntheticPngBase64(_intent: RequestCaptureIntent["intent"]): string {
  // Pre-encoded 1x1 transparent PNG.
  return "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
}
