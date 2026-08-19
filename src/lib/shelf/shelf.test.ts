import { render } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import ShelfCard from "./ShelfCard.svelte";
import ShelfQueue from "./ShelfQueue.svelte";
import type { ShelfQueueCard, ShelfQueueSnapshot, ShelfTimerState } from "$lib/ipc/types";

function makeCard(shelfId: string, title = "Example"): ShelfQueueCard {
  return {
    shelfId,
    captureId: `capture-${shelfId}`,
    pngPath: `/cache/${shelfId}/capture.png`,
    sizeBytes: 2048,
    createdAtMs: 1_700_000_000_000,
    bounds: {
      origin: { x: 0, y: 0 },
      size: { width: 320, height: 240 },
    },
    metadata: { title, note: "", tags: [] },
    timer: {
      addedAtElapsedMs: 0,
      deadlineAtElapsedMs: 60_000,
    } satisfies ShelfTimerState,
  };
}

function makeSnapshot(
  cards: ShelfQueueCard[],
  overflow: ShelfQueueCard[] = [],
): ShelfQueueSnapshot {
  return {
    cards,
    overflow,
    snapshotAtMs: 0,
  };
}

describe("ShelfCard", () => {
  it("renders the title and size when a card is supplied", () => {
    const { getByTestId } = render(ShelfCard, {
      card: makeCard("shelf-1"),
      nowMs: 1_000,
    });
    const card = getByTestId("shelf-card");
    expect(card.getAttribute("data-shelf-id")).toBe("shelf-1");
    expect(getByTestId("shelf-title").textContent).toBe("Example");
    expect(getByTestId("shelf-size").textContent).toContain("2 KB");
  });

  it("falls back to 'Untitled capture' when title is empty", () => {
    const card = makeCard("shelf-1", "");
    const { getByTestId } = render(ShelfCard, { card, nowMs: 0 });
    expect(getByTestId("shelf-title").textContent).toBe("Untitled capture");
  });

  it("invokes the dismiss callback with the shelf id", async () => {
    const onDismiss = vi.fn();
    const { getByTestId } = render(ShelfCard, {
      card: makeCard("shelf-1"),
      nowMs: 0,
      onDismiss,
    });
    const button = getByTestId("shelf-dismiss") as HTMLButtonElement;
    button.click();
    expect(onDismiss).toHaveBeenCalledWith("shelf-1");
  });

  it("invokes the copy and save-as callbacks", () => {
    const onCopy = vi.fn();
    const onSaveAs = vi.fn();
    const { getByTestId } = render(ShelfCard, {
      card: makeCard("shelf-2"),
      nowMs: 0,
      onCopy,
      onSaveAs,
    });
    (getByTestId("shelf-copy") as HTMLButtonElement).click();
    (getByTestId("shelf-save-as") as HTMLButtonElement).click();
    expect(onCopy).toHaveBeenCalledWith("shelf-2");
    expect(onSaveAs).toHaveBeenCalledWith("shelf-2");
  });

  it("invokes hover and unhover callbacks on mouse enter and leave", () => {
    const onHover = vi.fn();
    const onUnhover = vi.fn();
    const { getByTestId } = render(ShelfCard, {
      card: makeCard("shelf-3"),
      nowMs: 0,
      onHover,
      onUnhover,
    });
    const card = getByTestId("shelf-card") as HTMLElement;
    card.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    card.dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
    expect(onHover).toHaveBeenCalledWith("shelf-3");
    expect(onUnhover).toHaveBeenCalledWith("shelf-3");
  });

  it("renders the countdown text driven by `nowMs`", () => {
    const { getByTestId } = render(ShelfCard, {
      card: makeCard("shelf-4"),
      nowMs: 35_000,
    });
    // 60s - 35s = 25s remaining.
    expect(getByTestId("shelf-countdown").textContent).toBe("25s");
  });

  it("renders 'expired' when remaining time is zero", () => {
    const { getByTestId } = render(ShelfCard, {
      card: makeCard("shelf-5"),
      nowMs: 120_000,
    });
    expect(getByTestId("shelf-countdown").textContent).toBe("expired");
  });

  it("renders paused countdown when pausedRemainingMs is set", () => {
    const card = makeCard("shelf-6");
    card.timer = {
      addedAtElapsedMs: 0,
      deadlineAtElapsedMs: 60_000,
      pausedAtElapsedMs: 5_000,
      pausedRemainingMs: 12_000,
    };
    const { getByTestId } = render(ShelfCard, { card, nowMs: 99_999 });
    // While paused, the remaining is the captured value, not
    // deadline - now (which would be negative).
    expect(getByTestId("shelf-countdown").textContent).toBe("12s");
  });
});

describe("ShelfQueue", () => {
  it("renders nothing when snapshot is null", () => {
    const { queryByTestId } = render(ShelfQueue, { snapshot: null });
    expect(queryByTestId("shelf-queue")).toBeNull();
  });

  it("renders nothing when snapshot is empty", () => {
    const { queryByTestId } = render(ShelfQueue, {
      snapshot: makeSnapshot([]),
    });
    expect(queryByTestId("shelf-queue")).toBeNull();
  });

  it("renders one card per visible slot", () => {
    const cards = [makeCard("a"), makeCard("b"), makeCard("c"), makeCard("d")];
    const { queryAllByTestId } = render(ShelfQueue, {
      snapshot: makeSnapshot(cards),
    });
    expect(queryAllByTestId("shelf-card")).toHaveLength(4);
    expect(queryAllByTestId("shelf-card").map((el) => el.getAttribute("data-shelf-id"))).toEqual([
      "a",
      "b",
      "c",
      "d",
    ]);
  });

  it("renders an overflow toggle when overflow has cards", () => {
    const visible = [makeCard("a"), makeCard("b"), makeCard("c"), makeCard("d")];
    const overflow = [makeCard("e"), makeCard("f")];
    const { getByTestId } = render(ShelfQueue, {
      snapshot: makeSnapshot(visible, overflow),
    });
    const toggle = getByTestId("shelf-overflow");
    expect(toggle).toBeTruthy();
    // Overflow cards are not rendered until the toggle is clicked.
    expect(toggle.querySelectorAll('[data-testid="shelf-card"]')).toHaveLength(0);
  });

  it("does not render an overflow toggle when there is no overflow", () => {
    const { queryByTestId } = render(ShelfQueue, {
      snapshot: makeSnapshot([makeCard("a")]),
    });
    expect(queryByTestId("shelf-overflow")).toBeNull();
  });
});
