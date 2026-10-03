// Mount the main App against the mock IPC layer and verify the
// tray-intent -> capture -> cancel flow renders the expected UI.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";

// Mock the IPC commands module so the component pulls in the deterministic
// shell implementation instead of the Tauri runtime.
vi.mock("$lib/ipc/commands", async () => {
  const shell = await import("$lib/ipc/shell.svelte");
  return {
    requestCapture: shell.mockRequestCapture,
    requestCommit: shell.mockRequestCommit,
    requestCancel: shell.mockRequestCancel,
    getSessionSnapshot: shell.mockGetSessionSnapshot,
    getShelfQueueSnapshot: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { cards: [], overflow: [], snapshotAtMs: 0 } }),
    showShelfQueue: vi.fn().mockResolvedValue({
      status: "ok",
      data: { cards: [], overflow: [], snapshotAtMs: 0 },
    }),
    getShelfPreferences: shell.mockGetShelfPreferences,
    updateShelfPreferences: shell.mockUpdateShelfPreferences,
    getHotkeyBindings: shell.mockGetHotkeyBindings,
    updateHotkeyBindings: shell.mockUpdateHotkeyBindings,
    getHotkeyStatus: shell.mockGetHotkeyStatus,
    setHotkeyPaused: shell.mockSetHotkeyPaused,
  };
});

// Mock the @tauri-apps/api/event module so the App's onMount doesn't throw.
// The captured `listen` handler map lets tests fire backend events.
const eventHandlers = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    eventHandlers.set(name, handler);
    return Promise.resolve(() => {});
  }),
  emit: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("$lib/overlay/KonvaStage.svelte", () => ({ default: () => {} }));

import App from "./App.svelte";
import { __resetMock } from "$lib/ipc/shell.svelte";

describe("App", () => {
  beforeEach(() => {
    eventHandlers.clear();
    __resetMock();
  });

  it("shows recent screenshots as the primary surface", () => {
    render(App);
    expect(screen.getByRole("region", { name: "Recent screenshots" })).toBeInTheDocument();
  });

  it("starts an area capture from the main window", async () => {
    const user = userEvent.setup();
    render(App);
    await user.click(screen.getByRole("button", { name: /capture area/i }));
    const shell = await import("$lib/ipc/shell.svelte");
    const response = await shell.mockGetSessionSnapshot();
    expect(response.status === "ok" && response.data.state).toBe("selecting");
  });

  it("shows persisted and newly captured images in newest-first order", async () => {
    const user = userEvent.setup();
    render(App);
    await user.click(screen.getByRole("button", { name: /capture area/i }));
    const handler = eventHandlers.get("pixelgrab://shelf-queue-updated");
    const card = (shelfId: string) => ({
      shelfId,
      captureId: shelfId,
      pngPath: "/cache/capture.png",
      sizeBytes: 1024,
      createdAtMs: 0,
      bounds: { origin: { x: 0, y: 0 }, size: { width: 320, height: 240 } },
      metadata: { title: shelfId, note: "", tags: [] },
      timer: { addedAtElapsedMs: 0, deadlineAtElapsedMs: Number.MAX_SAFE_INTEGER },
    });
    handler!({ payload: { cards: [card("newest")], overflow: [card("older")], snapshotAtMs: 0 } });
    expect(
      (await screen.findAllByTestId("shelf-card")).map((node) =>
        node.getAttribute("data-shelf-id"),
      ),
    ).toEqual(["newest", "older"]);
  });

  // Tracer 15 closes the documentation-vs-implementation gap from
  // `docs/ACCESSIBILITY.md`: every <button> in the main window
  // must have either visible text content or an aria-label so a
  // screen reader can announce a name for it.
  it("every button has either visible text or an aria-label", () => {
    const { container } = render(App);
    const buttons = container.querySelectorAll("button");
    expect(buttons.length).toBeGreaterThan(0);
    for (const button of buttons) {
      const text = (button.textContent ?? "").trim();
      const ariaLabel = button.getAttribute("aria-label")?.trim() ?? "";
      expect(text.length + ariaLabel.length).toBeGreaterThan(0);
    }
  });

  it("announces a commit failure forwarded from the hidden overlay", async () => {
    render(App);
    const handler = eventHandlers.get("pixelgrab://commit-failed");
    expect(handler).toBeTruthy();
    handler!({ payload: { message: "Clipboard unavailable" } });
    expect(await screen.findByRole("alert")).toHaveTextContent("Clipboard unavailable");
  });

  // Issue #63: the shelf card's Edit action forwards the reopened
  // scene through `pixelgrab://revision-opened`; the main window must
  // mount the revision editor for it.
  it("mounts the revision editor when a reopen event arrives", async () => {
    render(App);
    const handler = eventHandlers.get("pixelgrab://revision-opened");
    expect(handler).toBeTruthy();
    handler!({
      payload: {
        shelfId: "shelf-1",
        captureId: "capture-1",
        pngPath: "/cache/shelf-1/capture.png",
        locks: ["shelf"],
        loaderStatus: "full",
        revision: {
          schemaVersion: 1,
          sourceShelfId: "shelf-1",
          sourceCaptureId: "capture-1",
          crop: { origin: { x: 0, y: 0 }, size: { width: 10, height: 10 } },
          size: { width: 10, height: 10 },
          annotations: [],
          badgeCounter: 2,
          activeTool: "arrow",
          activeColor: "red",
          activeStroke: "medium",
          metadata: { title: "Reopened", note: "", tags: [] },
        },
      },
    });
    expect(await screen.findByTestId("revision-editor")).toBeTruthy();
    expect(screen.getByTestId("revision-title")).toHaveValue("Reopened");
    expect(screen.getByTestId("open-settings")).toBeDisabled();
    eventHandlers.get("pixelgrab://revision-closed")!({ payload: null });
    expect(await screen.findByRole("region", { name: "Recent screenshots" })).toBeInTheDocument();
  });
});
