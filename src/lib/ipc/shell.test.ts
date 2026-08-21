// Synthetic end-to-end test that drives the frontend mock through the
// tray-intent -> IPC -> commit flow without a Tauri runtime. Issue #60
// collapsed the overlay reveal contract into one backend seam, so a
// successful capture lands in `Selecting` directly — there is no
// separate overlay IPC to call.

import { describe, it, expect, beforeEach } from "vitest";
import {
  __resetMock,
  mockRequestCancel,
  mockRequestCapture,
  mockRequestCommit,
  mockSessionState,
} from "./shell.svelte";

describe("synthetic capture end-to-end", () => {
  beforeEach(() => {
    __resetMock();
  });

  it("walks idle -> capturing -> selecting -> idle via the single reveal seam", async () => {
    expect(mockSessionState()).toBe("idle");

    const capture = await mockRequestCapture({ intent: "region" });
    expect(capture.status).toBe("ok");
    if (capture.status === "ok") {
      expect(capture.data.capture.captureId).toBeTruthy();
      expect(capture.data.diagnostics?.captureId).toBe(capture.data.capture.captureId);
      // Issue #60: the capture response now carries the overlay latency
      // because the overlay reveal contract is collapsed into one seam.
      expect(capture.data.diagnostics?.captureToOverlayMs).toBeTypeOf("number");
    }
    expect(mockSessionState()).toBe("selecting");

    const commit = await mockRequestCommit({
      crop: { origin: { x: 0, y: 0 }, size: { width: 100, height: 100 } },
      toShelf: true,
      toClipboard: true,
      saveAs: false,
    });
    expect(commit.status).toBe("ok");
    if (commit.status === "ok") {
      expect(commit.data.outcome.captureId).toBeTruthy();
      expect(commit.data.outcome.pngBytes).toBe(100 * 100 * 4);
    }
    expect(mockSessionState()).toBe("idle");
  });

  it("rejects concurrent capture requests", async () => {
    await mockRequestCapture({ intent: "region" });
    const second = await mockRequestCapture({ intent: "full_screen" });
    expect(second.status).toBe("err");
  });

  it("rejects commit when no selection is in progress", async () => {
    const commit = await mockRequestCommit({
      crop: { origin: { x: 0, y: 0 }, size: { width: 10, height: 10 } },
      toShelf: false,
      toClipboard: true,
      saveAs: false,
    });
    expect(commit.status).toBe("err");
  });

  it("Escape cancels a session in progress", async () => {
    await mockRequestCapture({ intent: "region" });
    expect(mockSessionState()).toBe("selecting");

    const result = await mockRequestCancel();
    expect(result.status).toBe("ok");
    if (result.status === "ok") {
      expect(result.data.action).toBe("session_cancelled");
    }
    expect(mockSessionState()).toBe("idle");
  });
});
