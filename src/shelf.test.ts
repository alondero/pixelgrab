// Regression coverage for issue #34: the shelf window must
// rehydrate from `get_shelf_queue_snapshot` on startup so a restart
// while the cache already holds entries renders the cards without
// waiting for the next `pixelgrab://shelf-queue-updated` event.
//
// The previous behaviour was event-only: `currentSnapshot` was
// initialised to `null` and only ever updated by the queue event,
// so the shelf rendered empty until the next commit happened to
// fire an event. Tracer 15 surfaces that gap because the validation
// criterion "A visible card protects its backing assets from
// deletion" cannot be inspected if the user cannot see the card.

import { describe, it, expect, vi, beforeEach } from "vitest";

// Mocks must be declared before the import under test. The
// bootstrap only touches `getShelfQueueSnapshot` and the
// `shelf-queue-updated` event, so we mock exactly those surfaces
// (and a no-op for the other IPC commands the shelf module
// imports) — keeping the test surface as small as possible so
// future additions to `$lib/ipc/commands` don't require mock
// edits here.
const mockGetShelfQueueSnapshot = vi.fn();
const mockGetShelfPreferences = vi.fn();
const mockListen = vi.fn();
// Issue #63 wiring mocks.
const mockOpenRevision = vi.fn();
const mockStartShelfDrag = vi.fn();
const mockShowMainWindow = vi.fn();
const mockEmit = vi.fn();
const mockPinStoreOpenPin = vi.fn();

vi.mock("$lib/ipc/commands", () => ({
  getShelfQueueSnapshot: mockGetShelfQueueSnapshot,
  getShelfPreferences: mockGetShelfPreferences,
  copyShelfCard: vi.fn(),
  saveShelfCardAs: vi.fn(),
  dismissCacheEntry: vi.fn(),
  hoverShelfCard: vi.fn(),
  unhoverShelfCard: vi.fn(),
  tickShelfQueue: vi.fn(),
  openRevision: mockOpenRevision,
  startShelfDrag: mockStartShelfDrag,
  showMainWindow: mockShowMainWindow,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mockListen,
  emit: mockEmit,
}));

vi.mock("$lib/pin/pinStore.svelte", () => ({
  pinStore: {
    openPin: mockPinStoreOpenPin,
  },
}));

import type { ShelfQueueCard, ShelfQueueSnapshot } from "$lib/ipc/types";

function makeCard(shelfId: string, title = "Rehydrated"): ShelfQueueCard {
  return {
    shelfId,
    captureId: `capture-${shelfId}`,
    pngPath: `/cache/${shelfId}/capture.png`,
    sizeBytes: 1024,
    createdAtMs: 1_700_000_000_000,
    bounds: { origin: { x: 0, y: 0 }, size: { width: 320, height: 240 } },
    metadata: { title, note: "", tags: [] },
    timer: { addedAtElapsedMs: 0, deadlineAtElapsedMs: 60_000 },
  };
}

const rehydratedSnapshot: ShelfQueueSnapshot = {
  cards: [makeCard("shelf-restored", "Persisted across restart")],
  overflow: [],
  snapshotAtMs: 0,
};

describe("shelf window bootstrap", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="shelf"></div>';
    mockListen.mockReset().mockResolvedValue(() => {});
    mockGetShelfQueueSnapshot
      .mockReset()
      .mockResolvedValue({ status: "ok", data: rehydratedSnapshot });
    mockGetShelfPreferences.mockReset().mockResolvedValue({
      status: "ok",
      data: { showCountdown: true },
    });
    vi.resetModules();
  });

  it("calls getShelfQueueSnapshot on init", async () => {
    await import("./shelf.svelte");
    // Flush microtasks so the async rehydration settles.
    await Promise.resolve();
    await Promise.resolve();

    expect(mockGetShelfQueueSnapshot).toHaveBeenCalledTimes(1);
  });

  it("renders a rehydrated card without any queue event being fired", async () => {
    await import("./shelf.svelte");
    // Flush microtasks so the async rehydration settles.
    await Promise.resolve();
    await Promise.resolve();

    const card = document.querySelector(
      '[data-testid="shelf-card"][data-shelf-id="shelf-restored"]',
    );
    expect(card).not.toBeNull();
    // The rehydrated title should be visible without any event ever
    // firing — that is the regression issue #34 captures.
    expect(document.querySelector('[data-testid="shelf-title"]')?.textContent).toBe(
      "Persisted across restart",
    );
  });

  it("still subscribes to the queue-updated event so live updates overwrite the seed", async () => {
    await import("./shelf.svelte");
    await Promise.resolve();

    // The "shelf-queue-updated" event must be subscribed to so the
    // live ticker keeps overwriting the seed after startup.
    expect(mockListen).toHaveBeenCalledWith(
      "pixelgrab://shelf-queue-updated",
      expect.any(Function),
    );
  });

  it("wires the card Pin action to pinStore.openPin with the card's source data", async () => {
    mockPinStoreOpenPin.mockReset().mockResolvedValue({ id: "pin-1" });
    await import("./shelf.svelte");
    await Promise.resolve();
    await Promise.resolve();

    const pin = document.querySelector<HTMLButtonElement>('[data-testid="shelf-pin"]');
    expect(pin).not.toBeNull();
    pin!.click();
    await Promise.resolve();
    await Promise.resolve();

    expect(mockPinStoreOpenPin).toHaveBeenCalledWith({
      captureId: "capture-shelf-restored",
      pngPath: "/cache/shelf-restored/capture.png",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 320, height: 240 } },
    });
  });

  it("wires the card Edit action to openRevision + revision-opened event + main window", async () => {
    mockOpenRevision.mockReset().mockResolvedValue({
      status: "ok",
      data: { context: { shelfId: "shelf-restored" } },
    });
    await import("./shelf.svelte");
    await Promise.resolve();
    await Promise.resolve();

    const edit = document.querySelector<HTMLButtonElement>('[data-testid="shelf-edit"]');
    expect(edit).not.toBeNull();
    edit!.click();
    // Flush all pending microtasks across the three-step async chain
    // (openRevision -> emit -> showMainWindow).
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(mockOpenRevision).toHaveBeenCalledWith({ shelfId: "shelf-restored" });
    expect(mockEmit).toHaveBeenCalledWith("pixelgrab://revision-opened", {
      shelfId: "shelf-restored",
    });
    expect(mockShowMainWindow).toHaveBeenCalledTimes(1);
  });

  it("wires the drag gesture to startShelfDrag with the card's shelf id", async () => {
    mockStartShelfDrag.mockReset().mockResolvedValue({
      status: "ok",
      data: { outcome: "cancelled", shouldDismiss: false },
    });
    await import("./shelf.svelte");
    await Promise.resolve();
    await Promise.resolve();

    const surface = document.querySelector<HTMLElement>('[data-testid="shelf-drag-surface"]');
    expect(surface).not.toBeNull();
    surface!.dispatchEvent(
      new MouseEvent("pointerdown", { button: 0, clientX: 10, clientY: 10, bubbles: true }),
    );
    surface!.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 60, clientY: 10, bubbles: true }),
    );
    await Promise.resolve();
    await Promise.resolve();

    expect(mockStartShelfDrag).toHaveBeenCalledWith({
      shelfId: "shelf-restored",
      dismissOnAccepted: true,
    });
  });

  it("does not hide the remaining queue when cleared follows an update", async () => {
    await import("./shelf.svelte");
    await Promise.resolve();

    const queueListener = mockListen.mock.calls.find(
      ([name]) => name === "pixelgrab://shelf-queue-updated",
    )?.[1] as ((event: { payload: ShelfQueueSnapshot }) => void) | undefined;
    const clearedListener = mockListen.mock.calls.find(
      ([name]) => name === "pixelgrab://shelf-cleared",
    )?.[1] as ((event: { payload: { shelfId: string } }) => void) | undefined;
    expect(queueListener).toBeDefined();
    expect(clearedListener).toBeDefined();

    const remaining = makeCard("shelf-remaining", "Still visible");
    queueListener?.({
      payload: { cards: [remaining], overflow: [], snapshotAtMs: 2 },
    });
    clearedListener?.({ payload: { shelfId: "shelf-dismissed" } });
    await Promise.resolve();

    expect(document.querySelectorAll('[data-testid="shelf-card"]')).toHaveLength(1);
    expect(document.querySelector('[data-shelf-id="shelf-remaining"]')).not.toBeNull();
  });

  it("does not let a slow startup preference overwrite a live update", async () => {
    type PreferenceResponse = {
      status: "ok";
      data: { showCountdown: boolean };
    };
    let resolvePreferences: ((value: PreferenceResponse) => void) | undefined;
    mockGetShelfPreferences.mockReturnValueOnce(
      new Promise<PreferenceResponse>((resolve) => {
        resolvePreferences = resolve;
      }),
    );

    await import("./shelf.svelte");
    const preferenceListener = mockListen.mock.calls.find(
      ([name]) => name === "pixelgrab://shelf-preferences-updated",
    )?.[1] as ((event: { payload: { showCountdown: boolean } }) => void) | undefined;
    expect(preferenceListener).toBeDefined();

    preferenceListener?.({ payload: { showCountdown: false } });
    resolvePreferences?.({ status: "ok", data: { showCountdown: true } });
    await Promise.resolve();
    await Promise.resolve();

    expect(document.querySelector('[data-testid="shelf-countdown"]')).toBeNull();
  });
});
