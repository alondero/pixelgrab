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
const mockListen = vi.fn();

vi.mock("$lib/ipc/commands", () => ({
  getShelfQueueSnapshot: mockGetShelfQueueSnapshot,
  copyShelfCard: vi.fn(),
  saveShelfCardAs: vi.fn(),
  dismissCacheEntry: vi.fn(),
  hoverShelfCard: vi.fn(),
  unhoverShelfCard: vi.fn(),
  tickShelfQueue: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mockListen,
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
});
