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

  it("ShelfCardView shape mirrors Rust", () => {
    // Mirrors `src-tauri/tests/ipc_contracts.rs::shelf_card_view_round_trips`.
    // The Rust struct lives in `src-tauri/src/shelf/mod.rs`; the TS mirror
    // is in `src/lib/shelf/types.ts` and re-uses `CacheEntryMetadata`
    // from the IPC types so the two sides cannot drift.
    const view: import("./types").ShelfCardView = {
      shelfId: "shelf-id",
      captureId: "capture-id",
      pngPath: "/cache/capture/capture.png",
      sizeBytes: 4096,
      createdAtMs: 1_700_000_000_000,
      bounds: { origin: { x: 0, y: 0 }, size: { width: 320, height: 240 } },
      metadata: {
        title: "Example",
        note: "first commit",
        tags: ["tracer-07"],
      },
    };
    const json = JSON.parse(JSON.stringify(view));
    expect(json.shelfId).toBe("shelf-id");
    expect(json.sizeBytes).toBe(4096);
    expect(json.metadata.tags).toEqual(["tracer-07"]);
  });

  it("ShelfQueueSnapshot carries cards, overflow, and timer", () => {
    // Mirrors the Rust contract in
    // `crates/pixelgrab-contracts/src/shelf_queue.rs`. The snapshot is
    // the canonical wire shape for the `pixelgrab://shelf-queue-updated`
    // event introduced by tracer 08.
    const snapshot: import("./types").ShelfQueueSnapshot = {
      cards: [
        {
          shelfId: "shelf-1",
          captureId: "capture-1",
          pngPath: "/cache/capture-1/capture.png",
          sizeBytes: 4096,
          createdAtMs: 1,
          bounds: { origin: { x: 0, y: 0 }, size: { width: 100, height: 80 } },
          metadata: { title: "Newest", note: "", tags: [] },
          timer: {
            addedAtElapsedMs: 0,
            deadlineAtElapsedMs: 60_000,
          },
        },
      ],
      overflow: [
        {
          shelfId: "shelf-2",
          captureId: "capture-2",
          pngPath: "/cache/capture-2/capture.png",
          sizeBytes: 4096,
          createdAtMs: 0,
          bounds: { origin: { x: 0, y: 0 }, size: { width: 100, height: 80 } },
          metadata: { title: "Older", note: "", tags: [] },
          timer: {
            addedAtElapsedMs: 0,
            deadlineAtElapsedMs: 60_000,
            pausedAtElapsedMs: 5_000,
            pausedRemainingMs: 12_000,
          },
        },
      ],
      snapshotAtMs: 7_777,
    };
    const json = JSON.parse(JSON.stringify(snapshot));
    expect(json.cards).toHaveLength(1);
    expect(json.overflow).toHaveLength(1);
    expect(json.cards[0].timer.deadlineAtElapsedMs).toBe(60_000);
    expect(json.overflow[0].timer.pausedRemainingMs).toBe(12_000);
    expect(json.snapshotAtMs).toBe(7_777);
  });

  it("CopyShelfCardRequest carries the shelf id", () => {
    const req: import("./types").CopyShelfCardRequest = { shelfId: "shelf-1" };
    const json = JSON.parse(JSON.stringify(req));
    expect(json.shelfId).toBe("shelf-1");
  });

  it("HoverShelfCardRequest carries the shelf id", () => {
    const req: import("./types").HoverShelfCardRequest = { shelfId: "shelf-2" };
    const json = JSON.parse(JSON.stringify(req));
    expect(json.shelfId).toBe("shelf-2");
  });
});
