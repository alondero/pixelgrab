import { render } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import ShelfCard from "./ShelfCard.svelte";
import type { ShelfCardView } from "./types";

function makeCard(): ShelfCardView {
  return {
    shelfId: "shelf-1",
    captureId: "capture-1",
    pngPath: "/cache/capture-1/capture.png",
    sizeBytes: 2048,
    createdAtMs: 1_700_000_000_000,
    bounds: {
      origin: { x: 0, y: 0 },
      size: { width: 320, height: 240 },
    },
    metadata: { title: "Example", note: "", tags: [] },
  };
}

describe("ShelfCard", () => {
  it("renders nothing when no card is supplied", () => {
    const { queryByTestId } = render(ShelfCard, { card: null });
    expect(queryByTestId("shelf-card")).toBeNull();
  });

  it("renders the title and size when a card is supplied", () => {
    const { getByTestId } = render(ShelfCard, { card: makeCard() });
    const card = getByTestId("shelf-card");
    expect(card.getAttribute("data-shelf-id")).toBe("shelf-1");
    expect(getByTestId("shelf-title").textContent).toBe("Example");
    expect(getByTestId("shelf-size").textContent).toContain("2 KB");
  });

  it("falls back to 'Untitled capture' when title is empty", () => {
    const card = makeCard();
    card.metadata.title = "";
    const { getByTestId } = render(ShelfCard, { card });
    expect(getByTestId("shelf-title").textContent).toBe("Untitled capture");
  });

  it("invokes the dismiss callback with the shelf id", async () => {
    const onDismiss = vi.fn();
    const { getByTestId } = render(ShelfCard, { card: makeCard(), onDismiss });
    const button = getByTestId("shelf-dismiss") as HTMLButtonElement;
    button.click();
    expect(onDismiss).toHaveBeenCalledWith("shelf-1");
  });
});
