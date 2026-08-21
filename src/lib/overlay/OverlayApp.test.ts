// Verify the overlay window's mount behaviour. Issue #60 collapsed the
// reveal contract into one backend seam (`show_over_virtual_desktop`
// → `overlay_mounted`), so the frontend's only job is to render the
// freeze frame. The backend pings `pixelgrab://overlay-revealed` and
// the overlay PULLS the capture via `get_session_snapshot` — a
// pull-after-ping cannot race listener registration the way a
// payload-bearing push event could (the tracer-15 freeze-frame bug).

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

// Capture the registered event listeners so tests can simulate a
// backend reveal ping.
const eventListeners = new Map<string, (event: unknown) => void>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: (event: unknown) => void) => {
    eventListeners.set(name, handler);
    return Promise.resolve(() => {
      eventListeners.delete(name);
    });
  }),
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
    eventListeners.clear();
    // The backend's `show_over_virtual_desktop` already walks
    // `Ready -> Selecting` before the overlay webview mounts, so the
    // session snapshot the frontend sees on mount is already in
    // `Selecting`.
    getSessionSnapshot.mockResolvedValue({
      status: "ok",
      data: {
        state: "selecting",
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

  it("reads the session snapshot on mount without driving the state machine", async () => {
    render(OverlayApp);
    await waitFor(() => {
      expect(getSessionSnapshot).toHaveBeenCalled();
    });
    // Issue #60 acceptance criterion: the overlay must not call
    // `requestOverlay`. The backend's `show_over_virtual_desktop`
    // seam advances the session before the webview mounts.
    expect(requestOverlay).not.toHaveBeenCalled();
  });

  it("registers the reveal listener before any await", async () => {
    render(OverlayApp);
    // The registration happens synchronously inside onMount — by the
    // time the first snapshot promise resolves, the listener must
    // already be recorded.
    await waitFor(() => {
      expect(eventListeners.has("pixelgrab://overlay-revealed")).toBe(true);
    });
  });

  it("re-pulls the snapshot when the backend emits a reveal ping", async () => {
    render(OverlayApp);
    await waitFor(() => {
      expect(eventListeners.has("pixelgrab://overlay-revealed")).toBe(true);
    });
    await waitFor(() => {
      expect(getSessionSnapshot).toHaveBeenCalledTimes(1);
    });
    const ping = eventListeners.get("pixelgrab://overlay-revealed");
    expect(ping).toBeDefined();
    getSessionSnapshot.mockClear();
    ping?.({ payload: null });
    await waitFor(() => {
      expect(getSessionSnapshot).toHaveBeenCalledTimes(1);
    });
  });
});
