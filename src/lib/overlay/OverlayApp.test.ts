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
import { render, waitFor } from "@testing-library/svelte";

// Track every IPC call so we can assert on the orchestration order,
// and capture event listeners so tests can fire backend events.
const requestOverlay = vi.fn();
const getSessionSnapshot = vi.fn();
const requestCommit = vi.fn();
const requestCancel = vi.fn();
const saveCaptureAs = vi.fn();

type Handler = (event: { payload: unknown }) => void;
const listeners = new Map<string, Handler[]>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: Handler) => {
    const list = listeners.get(name) ?? [];
    list.push(handler);
    listeners.set(name, list);
    return Promise.resolve(() => {});
  }),
}));

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
import type { CaptureResolutionDto } from "$lib/ipc/types";

function makeCapture(id: string): CaptureResolutionDto {
  return {
    format: "virtual_desktop",
    bounds: { origin: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } },
    assetUrl: `data:image/png;base64,${id}`,
    captureId: id,
    capturedAtMs: 1,
  };
}

function fire(name: string, payload: unknown): void {
  for (const handler of listeners.get(name) ?? []) {
    handler({ payload });
  }
}

describe("OverlayApp", () => {
  beforeEach(() => {
    listeners.clear();
    requestOverlay.mockReset();
    getSessionSnapshot.mockReset();
    requestCommit.mockReset();
    requestCancel.mockReset();
    saveCaptureAs.mockReset();
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

  it("adopts the capture announced by pixelgrab://capture-ready", async () => {
    // Regression for issue #63: the pre-allocated overlay webview
    // mounts once at boot with NO capture. Each reveal must push the
    // fresh capture into the page or the user sees the stale boot UI.
    const { screen } = await import("@testing-library/svelte");
    render(OverlayApp);
    await waitFor(() => {
      expect(listeners.has("pixelgrab://capture-ready")).toBe(true);
    });
    expect(screen.queryByTestId("diagnostics-id")).toBeNull();

    fire("pixelgrab://capture-ready", makeCapture("second-capture"));
    await waitFor(() => {
      expect(screen.getByTestId("diagnostics-id").textContent).toBe("second-capture");
    });
  });

  it("resets annotation state when a new capture starts", async () => {
    const { annotationStore } = await import("$lib/annotation/store.svelte");
    render(OverlayApp);
    await waitFor(() => {
      expect(listeners.has("pixelgrab://capture-ready")).toBe(true);
    });
    // Simulate leftover state from a previous session.
    annotationStore.setColor("green");
    fire("pixelgrab://capture-ready", makeCapture("third-capture"));
    await waitFor(() => {
      expect(annotationStore.color).toBe("red");
    });
  });
});
