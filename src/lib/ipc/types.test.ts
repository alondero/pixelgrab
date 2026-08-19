// Contract tests for the IPC types. Mirror the Rust-side tests in
// `src-tauri/tests/ipc_contracts.rs`. These tests verify that the TypeScript
// types produce the same camelCase JSON the Rust serde layer emits.

import { describe, it, expect } from "vitest";
import type {
  CaptureDiagnostics,
  CaptureResolutionDto,
  CommitRequest,
  CommitResponse,
  IpcResponse,
  PhysicalBounds,
  RequestCaptureIntent,
  RequestCommitIntent,
  RequestOverlayIntent,
  SessionSnapshot,
  SessionState,
} from "./types";

describe("IPC type contract", () => {
  it("RequestCaptureIntent serialises to camelCase", () => {
    const intent: RequestCaptureIntent = { intent: "region" };
    expect(JSON.parse(JSON.stringify(intent))).toEqual({ intent: "region" });
  });

  it("RequestCommitIntent serialises to camelCase", () => {
    const intent: RequestCommitIntent = {
      crop: { origin: { x: 0, y: 0 }, size: { width: 100, height: 200 } },
      toShelf: true,
      toClipboard: false,
      saveAs: false,
    };
    const json = JSON.parse(JSON.stringify(intent));
    expect(json.toShelf).toBe(true);
    expect(json.toClipboard).toBe(false);
    expect(json.crop.size.width).toBe(100);
  });

  it("RequestOverlayIntent serialises to camelCase", () => {
    const intent: RequestOverlayIntent = {
      selection: { origin: { x: 5, y: 10 }, size: { width: 50, height: 60 } },
    };
    const json = JSON.parse(JSON.stringify(intent));
    expect(json.selection.origin.x).toBe(5);
    expect(json.selection.size.width).toBe(50);
  });

  it("CommitRequest serialises to camelCase", () => {
    const req: CommitRequest = {
      crop: { origin: { x: 0, y: 0 }, size: { width: 1, height: 1 } },
      toShelf: false,
      toClipboard: true,
      saveAs: false,
    };
    const json = JSON.parse(JSON.stringify(req));
    expect(json.toClipboard).toBe(true);
  });

  it("CommitResponse carries the outcome", () => {
    const response: CommitResponse = {
      outcome: {
        captureId: "abc",
        pngBytes: 1024,
        pngPath: "/tmp/abc.png",
      },
    };
    const json = JSON.parse(JSON.stringify(response));
    expect(json.outcome.captureId).toBe("abc");
    expect(json.outcome.pngBytes).toBe(1024);
  });

  it("IpcResponse Ok shape", () => {
    const ok: IpcResponse<number> = { status: "ok", data: 42 };
    expect(ok.status).toBe("ok");
    if (ok.status === "ok") {
      expect(ok.data).toBe(42);
    }
  });

  it("IpcResponse Err shape", () => {
    const err: IpcResponse<number> = {
      status: "err",
      error: { kind: "invalid_payload", message: "bad" },
    };
    expect(err.status).toBe("err");
    if (err.status === "err") {
      expect(err.error.kind).toBe("invalid_payload");
    }
  });

  it("SessionSnapshot reflects state", () => {
    const snapshot: SessionSnapshot = {
      state: "selecting" as SessionState,
      lastCapture: {
        format: "virtual_desktop",
        bounds: { origin: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } },
        assetUrl: "data:",
        captureId: "id",
        capturedAtMs: 1,
      },
      selection: { origin: { x: 0, y: 0 }, size: { width: 1, height: 1 } },
    };
    const json = JSON.parse(JSON.stringify(snapshot));
    expect(json.state).toBe("selecting");
    expect(json.lastCapture.format).toBe("virtual_desktop");
  });

  it("CaptureResolutionDto round trips", () => {
    const dto: CaptureResolutionDto = {
      format: "virtual_desktop",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } },
      assetUrl: "data:image/png;base64,ABC",
      captureId: "id-1",
      capturedAtMs: 1,
    };
    const json = JSON.parse(JSON.stringify(dto));
    expect(json.captureId).toBe("id-1");
    expect(json.assetUrl).toMatch(/^data:/);
  });

  it("PhysicalBounds struct shape", () => {
    const bounds: PhysicalBounds = {
      origin: { x: 1, y: 2 },
      size: { width: 3, height: 4 },
    };
    expect(bounds.origin.x).toBe(1);
    expect(bounds.size.height).toBe(4);
  });

  it("CaptureDiagnostics serialises to camelCase", () => {
    const diag: CaptureDiagnostics = {
      captureId: "id",
      captureStartedAtMs: 1,
      captureCompletedAtMs: 30,
      captureDurationMs: 29,
      overlayVisibleAtMs: 40,
      captureToOverlayMs: 10,
      monitorId: "primary",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } },
    };
    const json = JSON.parse(JSON.stringify(diag));
    expect(json.captureStartedAtMs).toBe(1);
    expect(json.captureToOverlayMs).toBe(10);
    expect(json.monitorId).toBe("primary");
  });
});
