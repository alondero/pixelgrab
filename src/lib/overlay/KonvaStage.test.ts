// Verify the KonvaStage component renders the stage container element. The
// actual Konva stage requires HTMLCanvasElement.getContext which is not
// implemented in jsdom. We mock the Konva module so the wrapper still
// exercises the Svelte reactivity surface without touching the canvas.

import { describe, it, expect, vi } from "vitest";

vi.mock("konva", () => {
  class FakeStage {
    add() {}
    on() {}
    destroy() {}
    width() {
      return 100;
    }
    height() {
      return 100;
    }
    getPointerPosition() {
      return { x: 0, y: 0 };
    }
  }
  class FakeLayer {
    add() {}
    draw() {}
    destroy() {}
  }
  class FakeImage {
    _set = vi.fn();
    on() {}
    width() {
      return 0;
    }
    height() {
      return 0;
    }
  }
  class FakeRect {
    x(_v?: number) {
      return 0;
    }
    y(_v?: number) {
      return 0;
    }
    width(_v?: number) {
      return 0;
    }
    height(_v?: number) {
      return 0;
    }
    position(_v?: { x: number; y: number }) {}
    size(_v?: { width: number; height: number }) {}
    visible(_v?: boolean) {}
    points(_v?: number[]) {}
  }
  class FakeLine {
    points(_v?: number[]) {}
    visible(_v?: boolean) {}
  }
  return {
    default: {
      Stage: FakeStage,
      Layer: FakeLayer,
      Image: FakeImage,
      Rect: FakeRect,
      Line: FakeLine,
    },
  };
});

import { render } from "@testing-library/svelte";
import KonvaStage from "./KonvaStage.svelte";

describe("KonvaStage", () => {
  it("renders the stage container", () => {
    const { container } = render(KonvaStage, {
      props: {
        assetUrl: "data:image/png;base64,AAAA",
        bounds: { origin: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } },
        stageWidth: 960,
        stageHeight: 540,
        onSelectionChange: () => {},
      },
    });
    const stage = container.querySelector('[data-testid="konva-stage"]');
    expect(stage).toBeInTheDocument();
  });
});
