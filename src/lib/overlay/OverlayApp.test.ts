// Verify the overlay window's mount behaviour: it must advance the
// session from `ready` to `selecting` by calling `requestOverlay` so
// the commit pipeline (which requires `Selecting`) can accept the
// user's crop. Without this call the session stays in `Ready` and
// every subsequent capture request is rejected with
// "cannot start capture: session is already Ready" — a regression
// that surfaced when the user pressed the hotkey and got a silent
// no-op, then tried the tray menu.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor } from "@testing-library/svelte";

// Track every IPC call so we can assert on the orchestration order.
const requestOverlay = vi.fn();
const getSessionSnapshot = vi.fn();
const requestCommit = vi.fn();
const requestCancel = vi.fn();
const saveCaptureAs = vi.fn();

vi.mock("$lib/ipc/commands", () => ({
  requestOverlay: (...args: unknown[]) => requestOverlay(...args),
  getSessionSnapshot: (...args: unknown[]) => getSessionSnapshot(...args),
  requestCommit: (...args: unknown[]) => requestCommit(...args),
  requestCancel: (...args: unknown[]) => requestCancel(...args),
  saveCaptureAs: (...args: unknown[]) => saveCaptureAs(...args),
}));

// Konva requires an HTMLCanvasElement.getContext that jsdom does not
// implement. Stub the stage out so the overlay mount still exercises
// the lifecycle above the canvas.
vi.mock("$lib/overlay/KonvaStage.svelte", () => ({
  default: () => {},
}));

import OverlayApp from "./OverlayApp.svelte";

describe("OverlayApp", () => {
  beforeEach(() => {
    requestOverlay.mockReset();
    getSessionSnapshot.mockReset();
    requestCommit.mockReset();
    requestCancel.mockReset();
    saveCaptureAs.mockReset();
    // The backend's `request_overlay` handler advances the session
    // from `ready` to `selecting` and stores the reported selection
    // bounds. The mock mirrors that contract.
    requestOverlay.mockResolvedValue({
      status: "ok",
      data: {
        snapshot: {
          state: "selecting",
          lastCapture: undefined,
          selection: {
            origin: { x: 0, y: 0 },
            size: { width: 0, height: 0 },
          },
        },
        diagnostics: null,
      },
    });
    getSessionSnapshot.mockResolvedValue({
      status: "ok",
      data: {
        state: "ready",
        lastCapture: {
          format: "virtual_desktop",
          bounds: {
            origin: { x: 0, y: 0 },
            size: { width: 1920, height: 1080 },
          },
          assetUrl: "data:image/png;base64,AAAA",
          captureId: "test-capture",
          capturedAtMs: 1,
        },
        selection: null,
      },
    });
  });

  it("calls requestOverlay on mount so the session advances to selecting", async () => {
    render(OverlayApp);
    await waitFor(() => {
      expect(requestOverlay).toHaveBeenCalledTimes(1);
    });
    // The selection passed to requestOverlay must have zero size —
    // the overlay reports the empty selection on mount and the user
    // updates it once they drag a region.
    const [intent] = requestOverlay.mock.calls[0];
    expect(intent.selection.size.width).toBe(0);
    expect(intent.selection.size.height).toBe(0);
  });
});
