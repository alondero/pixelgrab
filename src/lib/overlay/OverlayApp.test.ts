// Verify the overlay window's mount + live-capture behaviour.
//
// Issue #60 collapsed the reveal contract into one backend seam
// (`show_over_virtual_desktop` → `overlay_mounted`), so the frontend's
// only job on mount is to read the snapshot — it never has to drive a
// `Ready -> Selecting` transition.
//
// Issue #63 regression: the overlay webview is pre-allocated at boot
// and stays alive (hidden) between captures. A mount-time-only
// snapshot read means the SECOND capture never reaches the page — the
// window is revealed with stale "No capture yet" content and the
// session wedges in `Selecting`. The backend therefore emits
// `pixelgrab://capture-ready` with the fresh capture; these tests pin
// that contract.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/svelte";

// Track every IPC call so we can assert on the orchestration order.
const requestOverlay = vi.fn();
const getSessionSnapshot = vi.fn();
const requestCommit = vi.fn();
const requestCancel = vi.fn();
const saveCaptureAs = vi.fn();
const showMainWindow = vi.fn();
const listen = vi.fn();

// The mock both records `listen` calls (for assertion) and registers
// handlers into a map so tests can fire backend events directly.
type Handler = (event: { payload: unknown }) => void;
const listeners = new Map<string, Handler[]>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listen(...args),
  emit: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("$lib/ipc/commands", () => ({
  requestOverlay: (...args: unknown[]) => requestOverlay(...args),
  getSessionSnapshot: (...args: unknown[]) => getSessionSnapshot(...args),
  requestCommit: (...args: unknown[]) => requestCommit(...args),
  requestCancel: (...args: unknown[]) => requestCancel(...args),
  saveCaptureAs: (...args: unknown[]) => saveCaptureAs(...args),
  showMainWindow: (...args: unknown[]) => showMainWindow(...args),
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
    listeners.clear();
    requestOverlay.mockReset();
    getSessionSnapshot.mockReset();
    requestCommit.mockReset();
    requestCancel.mockReset();
    saveCaptureAs.mockReset();
    showMainWindow.mockReset().mockResolvedValue({ status: "ok", data: null });
    listen.mockReset().mockImplementation((name: string, handler: Handler) => {
      const list = listeners.get(name) ?? [];
      list.push(handler);
      listeners.set(name, list);
      return Promise.resolve(() => {});
    });
    // Boot-time snapshot: no capture has happened yet.
    getSessionSnapshot.mockResolvedValue({
      status: "ok",
      data: { state: "idle", lastCapture: undefined, selection: null },
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

  it("hydrates a pre-mounted overlay when native capture-ready arrives", async () => {
    render(OverlayApp);
    await waitFor(() => {
      expect(listen).toHaveBeenCalledWith("pixelgrab://capture-ready", expect.any(Function));
    });
    const handler = listen.mock.calls[0][1] as (event: {
      payload: {
        capture: {
          format: "virtual_desktop";
          bounds: { origin: { x: number; y: number }; size: { width: number; height: number } };
          assetUrl: string;
          captureId: string;
          capturedAtMs: number;
        };
      };
    }) => void;
    handler({
      payload: {
        capture: {
          format: "virtual_desktop",
          bounds: { origin: { x: 0, y: 0 }, size: { width: 800, height: 600 } },
          assetUrl: "data:image/png;base64,AAAA",
          captureId: "event-capture",
          capturedAtMs: 2,
        },
      },
    });
    await waitFor(() => {
      expect(screen.getByTestId("diagnostics-id")).toHaveTextContent("event-capture");
    });
  });
});
