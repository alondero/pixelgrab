// Synthetic end-to-end test that drives the frontend mock through the
// tray-intent -> IPC -> overlay -> commit flow without a Tauri runtime.

import { describe, it, expect, beforeEach } from "vitest";
import {
  __resetMock,
  mockRequestCapture,
  mockRequestCommit,
  mockRequestOverlay,
  mockSessionState,
} from "./shell.svelte";

describe("synthetic capture end-to-end", () => {
  beforeEach(() => {
    __resetMock();
  });

  it("walks idle -> capturing -> ready -> selecting -> idle", async () => {
    expect(mockSessionState()).toBe("idle");

    const capture = await mockRequestCapture({ intent: "region" });
    expect(capture.status).toBe("ok");
    if (capture.status === "ok") {
      expect(capture.data.captureId).toBeTruthy();
    }
    expect(mockSessionState()).toBe("ready");

    const overlay = await mockRequestOverlay({
      selection: { origin: { x: 0, y: 0 }, size: { width: 100, height: 100 } },
    });
    expect(overlay.status).toBe("ok");
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
});
