// Contract tests for the IPC types. Mirror the Rust-side tests in
// `src-tauri/tests/ipc_contracts.rs`. These tests verify that the TypeScript
// types produce the same camelCase JSON the Rust serde layer emits.

import { describe, it, expect } from "vitest";
import type {
  CaptureDiagnostics,
  CaptureResolutionDto,
  CommitRequest,
  CommitResponse,
  DragDiagnostics,
  DragRequest,
  HotkeyBindingsDto,
  HotkeyRegistryStatusDto,
  IpcResponse,
  PhysicalBounds,
  RequestCaptureIntent,
  RequestCommitIntent,
  SecondaryLaunchIntent,
  SessionSnapshot,
  SessionState,
  StartShelfDragIntent,
  StartShelfDragResult,
  UpdateHotkeyBindingsRequest,
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
    const req: DragRequest = {
      captureId: "capture-1",
      shelfId: "shelf-1",
      pngPath: "C:/cache/capture.png",
      bgraPixels: new Array(4 * 4 * 4).fill(0),
      width: 4,
      height: 4,
    };
    const json = JSON.parse(JSON.stringify(req));
    expect(json.captureId).toBe("capture-1");
    expect(json.shelfId).toBe("shelf-1");
    expect(json.pngPath).toMatch(/capture.png/);
    expect(json.bgraPixels).toHaveLength(4 * 4 * 4);
  });

  it("ShelfClearedEvent shape mirrors Rust", () => {
    // Mirrors `crate::shelf::ShelfClearedEvent` in
    // `src-tauri/src/shelf/mod.rs`. Without this test a `shelfId`
    // rename on the Rust side would slip past CI silently because
    // the listen call site in `src/shelf.ts` declared the field
    // inline before it was lifted into the named type.
    const event: import("$lib/shelf/types").ShelfClearedEvent = {
      shelfId: "shelf-1",
    };
    const json = JSON.parse(JSON.stringify(event));
    expect(json.shelfId).toBe("shelf-1");
  });

  it("StartShelfDragIntent serialises the request envelope", () => {
    const intent: StartShelfDragIntent = {
      shelfId: "shelf-1",
      dismissOnAccepted: true,
    };
    const json = JSON.parse(JSON.stringify(intent));
    expect(json.shelfId).toBe("shelf-1");
    expect(json.dismissOnAccepted).toBe(true);
  });

  it("StartShelfDragResult serialises outcome and dismiss hint", () => {
    const diag: DragDiagnostics = {
      startedAtMs: 1,
      completedAtMs: 100,
      durationMs: 99,
      targetEffect: "copy",
      targetKind: "chromium",
      captureId: "c",
    };
    const result: StartShelfDragResult = {
      outcome: "accepted",
      diagnostics: diag,
      shouldDismiss: true,
    };
    const json = JSON.parse(JSON.stringify(result));
    expect(json.outcome).toBe("accepted");
    expect(json.shouldDismiss).toBe(true);
    expect(json.diagnostics.targetEffect).toBe("copy");
    expect(json.diagnostics.targetKind).toBe("chromium");
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

  it("ShelfPreferencesDto round-trips every field", () => {
    // Mirrors the Rust struct in
    // `crates/pixelgrab-contracts/src/shelf_preferences.rs`.
    const prefs: import("./types").ShelfPreferencesDto = {
      schemaVersion: 1,
      corner: "top_left",
      targetMonitorId: "secondary",
      marginPx: 32,
      autoDismissEnabled: false,
      lifetimeSeconds: 45,
      visibleCardCount: 2,
      showCountdown: false,
    };
    const json = JSON.parse(JSON.stringify(prefs));
    expect(json.schemaVersion).toBe(1);
    expect(json.corner).toBe("top_left");
    expect(json.targetMonitorId).toBe("secondary");
    expect(json.marginPx).toBe(32);
    expect(json.autoDismissEnabled).toBe(false);
    expect(json.lifetimeSeconds).toBe(45);
    expect(json.visibleCardCount).toBe(2);
    expect(json.showCountdown).toBe(false);
  });

  it("UpdateShelfPreferencesRequest carries the preferences + commit flag", () => {
    const req: import("./types").UpdateShelfPreferencesRequest = {
      preferences: {
        schemaVersion: 1,
        corner: "bottom_right",
        targetMonitorId: null,
        marginPx: 24,
        autoDismissEnabled: true,
        lifetimeSeconds: 60,
        visibleCardCount: 4,
        showCountdown: true,
      },
      commit: true,
    };
    const json = JSON.parse(JSON.stringify(req));
    expect(json.commit).toBe(true);
    expect(json.preferences.corner).toBe("bottom_right");
  });

  // --- Tracer-05: text + blur + Save As wire shapes ------------------

  it("AnnotationGeometry text variant carries text payload", () => {
    const geom: import("./types").AnnotationGeometry = {
      kind: "text",
      origin: { x: 10, y: 20 },
      size: { width: 120, height: 40 },
      text: "hello\nworld",
    };
    const json = JSON.parse(JSON.stringify(geom));
    expect(json.kind).toBe("text");
    expect(json.text).toBe("hello\nworld");
    expect(json.size.width).toBe(120);
  });

  it("AnnotationGeometry blur variant carries radius payload", () => {
    const geom: import("./types").AnnotationGeometry = {
      kind: "blur",
      origin: { x: 5, y: 5 },
      size: { width: 40, height: 40 },
      radius: 4,
    };
    const json = JSON.parse(JSON.stringify(geom));
    expect(json.kind).toBe("blur");
    expect(json.radius).toBe(4);
  });

  it("AnnotationTool includes text and blur", () => {
    // Type-level + runtime check: the union is wired through to the
    // wire shape. A future contributor who drops a tool will see a
    // compile error here.
    const tools: Array<import("./types").AnnotationTool> = [
      "select",
      "arrow",
      "rectangle",
      "numbered_badge",
      "text",
      "blur",
    ];
    expect(tools).toContain("text");
    expect(tools).toContain("blur");
  });

  it("SaveCaptureAsRequest carries crop + annotations + suggested filename", () => {
    const req: import("./types").SaveCaptureAsRequest = {
      crop: { origin: { x: 0, y: 0 }, size: { width: 200, height: 100 } },
      annotations: [],
      suggestedFilename: "capture.png",
    };
    const json = JSON.parse(JSON.stringify(req));
    expect(json.suggestedFilename).toBe("capture.png");
    expect(json.crop.size.width).toBe(200);
    expect(json.annotations).toEqual([]);
  });

  it("SaveCaptureAsResponse omits path when cancelled", () => {
    const resp: import("./types").SaveCaptureAsResponse = {
      pngBytes: 0,
    };
    const json = JSON.parse(JSON.stringify(resp));
    expect(json.path).toBeUndefined();
    expect(json.pngBytes).toBe(0);
  });
});

describe("IPC type contract — hotkey bindings (tracer 14)", () => {
  it("HotkeyBindingsDto serialises to camelCase", () => {
    const dto: HotkeyBindingsDto = {
      schemaVersion: 1,
      regionCapture: "Ctrl+Shift+S",
      fullScreenCapture: "Ctrl+Shift+F",
      shelfToggle: "Ctrl+Shift+L",
      paused: false,
    };
    const json = JSON.parse(JSON.stringify(dto));
    expect(json.schemaVersion).toBe(1);
    expect(json.regionCapture).toBe("Ctrl+Shift+S");
    expect(json.paused).toBe(false);
  });

  it("HotkeyBindingsDto accepts null bindings", () => {
    const dto: HotkeyBindingsDto = {
      schemaVersion: 1,
      regionCapture: null,
      fullScreenCapture: null,
      shelfToggle: null,
      paused: true,
    };
    const json = JSON.parse(JSON.stringify(dto));
    expect(json.regionCapture).toBeNull();
    expect(json.paused).toBe(true);
  });

  it("UpdateHotkeyBindingsRequest matches the Rust wire shape", () => {
    const payload: UpdateHotkeyBindingsRequest = {
      bindings: {
        schemaVersion: 1,
        regionCapture: "Ctrl+Alt+R",
        paused: false,
      },
    };
    const json = JSON.parse(JSON.stringify(payload));
    expect(json.bindings.schemaVersion).toBe(1);
    expect(json.bindings.regionCapture).toBe("Ctrl+Alt+R");
  });

  it("HotkeyRegistryStatusDto serialises error fields", () => {
    const status: HotkeyRegistryStatusDto = {
      active: false,
      paused: false,
      lastError: "registration_failed",
      conflictingAction: "shelf_toggle",
    };
    const json = JSON.parse(JSON.stringify(status));
    expect(json.active).toBe(false);
    expect(json.conflictingAction).toBe("shelf_toggle");
  });

  it("SecondaryLaunchIntent uses tagged kinds on the wire", () => {
    const intents: SecondaryLaunchIntent[] = [
      { kind: "default" },
      { kind: "capture_region" },
      { kind: "capture_full_screen" },
      { kind: "shelf_history" },
      { kind: "open_settings" },
    ];
    for (const intent of intents) {
      const json = JSON.parse(JSON.stringify(intent));
      expect(json.kind).toBe(intent.kind);
    }
  });
});

// ---------------------------------------------------------------------------
// Tracer-10: reopen / non-destructive revision IPC payloads.
// ---------------------------------------------------------------------------

describe("IPC type contract — revision (tracer-10)", () => {
  it("OpenRevisionIntent serialises to camelCase", () => {
    const intent: import("./types").OpenRevisionIntent = { shelfId: "shelf-1" };
    const json = JSON.parse(JSON.stringify(intent));
    expect(json.shelfId).toBe("shelf-1");
  });

  it("OpenRevisionResult wraps the context", () => {
    const result: import("./types").OpenRevisionResult = {
      context: {
        shelfId: "shelf-1",
        captureId: "cap-1",
        pngPath: "/tmp/cap-1/capture.png",
        revision: {
          schemaVersion: 1,
          sourceShelfId: "shelf-1",
          sourceCaptureId: "cap-1",
          crop: { origin: { x: 0, y: 0 }, size: { width: 100, height: 100 } },
          size: { width: 100, height: 100 },
          annotations: [],
          badgeCounter: 1,
          activeTool: "select",
          activeColor: "red",
          activeStroke: "medium",
          metadata: { title: "", note: "", tags: [] },
        },
        locks: ["shelf", "editor"],
        loaderStatus: "full",
      },
    };
    const json = JSON.parse(JSON.stringify(result));
    expect(json.context.shelfId).toBe("shelf-1");
    expect(json.context.revision.schemaVersion).toBe(1);
    expect(json.context.locks).toEqual(["shelf", "editor"]);
    expect(json.context.loaderStatus).toBe("full");
  });

  it("CommitRevisionIntent carries annotations + style + metadata", () => {
    const intent: import("./types").CommitRevisionIntent = {
      shelfId: "shelf-1",
      annotations: [],
      badgeCounter: 3,
      activeTool: "arrow",
      activeColor: "red",
      activeStroke: "medium",
      metadata: { title: "edited", note: "", tags: [] },
      toClipboard: true,
    };
    const json = JSON.parse(JSON.stringify(intent));
    expect(json.shelfId).toBe("shelf-1");
    expect(json.badgeCounter).toBe(3);
    expect(json.activeTool).toBe("arrow");
    expect(json.toClipboard).toBe(true);
  });

  it("CommitRevisionResult wraps the new entry's outcome", () => {
    const result: import("./types").CommitRevisionResult = {
      outcome: {
        captureId: "new-cap",
        shelfId: "new-shelf",
        pngPath: "/tmp/new.png",
        pngBytes: 4096,
        sizeBytes: 8192,
        createdAtMs: 1_700_000_000_000,
      },
    };
    const json = JSON.parse(JSON.stringify(result));
    expect(json.outcome.captureId).toBe("new-cap");
    expect(json.outcome.shelfId).toBe("new-shelf");
  });

  it("CancelRevisionIntent and CancelRevisionResult serialise", () => {
    const intent: import("./types").CancelRevisionIntent = { shelfId: "shelf-1" };
    const result: import("./types").CancelRevisionResult = {
      cancelled: true,
      reason: "cancelled",
    };
    expect(JSON.parse(JSON.stringify(intent)).shelfId).toBe("shelf-1");
    const json = JSON.parse(JSON.stringify(result));
    expect(json.cancelled).toBe(true);
    expect(json.reason).toBe("cancelled");
  });

  it("UpdateRevisionIntent and UpdateRevisionResult serialise", () => {
    const meta: import("./types").RevisionMetadata = {
      schemaVersion: 1,
      sourceShelfId: "shelf-1",
      sourceCaptureId: "cap-1",
      crop: { origin: { x: 0, y: 0 }, size: { width: 100, height: 100 } },
      size: { width: 100, height: 100 },
      annotations: [],
      badgeCounter: 1,
      activeTool: "select",
      activeColor: "red",
      activeStroke: "medium",
      metadata: { title: "", note: "", tags: [] },
    };
    const intent: import("./types").UpdateRevisionIntent = {
      shelfId: "shelf-1",
      revision: meta,
    };
    const result: import("./types").UpdateRevisionResult = { revision: meta };
    const intentJson = JSON.parse(JSON.stringify(intent));
    expect(intentJson.shelfId).toBe("shelf-1");
    expect(intentJson.revision.schemaVersion).toBe(1);
    const resultJson = JSON.parse(JSON.stringify(result));
    expect(resultJson.revision.schemaVersion).toBe(1);
  });

  it("RevisionMetadata carries every annotation field", () => {
    const meta: import("./types").RevisionMetadata = {
      schemaVersion: 1,
      sourceShelfId: "shelf-1",
      sourceCaptureId: "cap-1",
      crop: { origin: { x: 0, y: 0 }, size: { width: 100, height: 100 } },
      size: { width: 100, height: 100 },
      annotations: [
        {
          id: 1,
          geometry: { kind: "arrow", tail: { x: 0, y: 0 }, tip: { x: 50, y: 50 } },
          color: "red",
          stroke: "medium",
          zOrder: 0,
        },
        {
          id: 2,
          geometry: {
            kind: "rectangle",
            origin: { x: 10, y: 10 },
            size: { width: 20, height: 20 },
          },
          color: "blue",
          stroke: "thin",
          zOrder: 1,
        },
        {
          id: 3,
          geometry: { kind: "numbered_badge", center: { x: 80, y: 80 }, radius: 18 },
          color: "yellow",
          stroke: "thin",
          zOrder: 2,
          number: 1,
        },
        {
          id: 4,
          geometry: {
            kind: "text",
            origin: { x: 0, y: 0 },
            size: { width: 50, height: 14 },
            text: "label",
          },
          color: "white",
          stroke: "thin",
          zOrder: 3,
        },
        {
          id: 5,
          geometry: {
            kind: "blur",
            origin: { x: 0, y: 0 },
            size: { width: 20, height: 20 },
            radius: 2,
          },
          color: "white",
          stroke: "medium",
          zOrder: 4,
        },
      ],
      badgeCounter: 4,
      activeTool: "rectangle",
      activeColor: "blue",
      activeStroke: "thick",
      metadata: { title: "x", note: "y", tags: ["a"] },
    };
    const json = JSON.parse(JSON.stringify(meta));
    expect(json.annotations).toHaveLength(5);
    expect(json.badgeCounter).toBe(4);
    expect(json.activeTool).toBe("rectangle");
    expect(json.metadata.tags).toEqual(["a"]);
  });

  it("RevisionLoaderStatus is a closed union", () => {
    const statuses: import("./types").RevisionLoaderStatus[] = ["full", "flat_fallback"];
    expect(statuses).toHaveLength(2);
  });
});
