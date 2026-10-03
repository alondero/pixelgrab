// Verify the KonvaStage component renders the stage container element. The
// actual Konva stage requires HTMLCanvasElement.getContext which is not
// implemented in jsdom. We mock the Konva module so the wrapper still
// exercises the Svelte reactivity surface without touching the canvas.

import { describe, it, expect, vi, beforeEach } from "vitest";

const stageSize = vi.hoisted(() => vi.fn());
const pointer = vi.hoisted(() => ({ x: 0, y: 0 }));
const fakeImage = vi.hoisted(() => ({ current: null as unknown }));
const stageEvents = vi.hoisted(
  () =>
    new Map<
      string,
      (event: {
        target?: unknown;
        evt: { ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean };
      }) => void
    >(),
);

vi.mock("konva", () => {
  class FakeStage {
    add() {}
    on(
      name: string,
      handler: (event: {
        target?: unknown;
        evt: { ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean };
      }) => void,
    ) {
      stageEvents.set(name, handler);
    }
    destroy() {}
    width() {
      return 100;
    }
    height() {
      return 100;
    }
    size(value: { width: number; height: number }) {
      stageSize(value);
    }
    getPointerPosition() {
      return { ...pointer };
    }
  }
  class FakeLayer {
    add() {}
    draw() {}
    destroy() {}
    destroyChildren() {}
    position(_v?: { x: number; y: number }) {}
    scale(_v?: { x: number; y: number }) {}
  }
  class FakeImage {
    constructor() {
      fakeImage.current = this;
    }
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
    rect = { x: 0, y: 0, width: 0, height: 0 };
    shown = false;
    x(_v?: number) {
      return this.rect.x;
    }
    y(_v?: number) {
      return this.rect.y;
    }
    width(_v?: number) {
      return this.rect.width;
    }
    height(_v?: number) {
      return this.rect.height;
    }
    position(value: { x: number; y: number }) {
      Object.assign(this.rect, value);
    }
    size(value: { width: number; height: number }) {
      Object.assign(this.rect, value);
    }
    visible(value?: boolean) {
      if (value !== undefined) this.shown = value;
      return this.shown;
    }
    points(_v?: number[]) {}
  }
  class FakeLine {
    points(_v?: number[]) {}
    visible(_v?: boolean) {}
  }
  class FakeGroup {
    add() {}
  }
  class FakeCircle {}
  class FakeText {}
  return {
    default: {
      Stage: FakeStage,
      Layer: FakeLayer,
      Image: FakeImage,
      Rect: FakeRect,
      Line: FakeLine,
      Group: FakeGroup,
      Circle: FakeCircle,
      Text: FakeText,
    },
  };
});

import { render } from "@testing-library/svelte";
import KonvaStage from "./KonvaStage.svelte";
import { annotationStore } from "$lib/annotation/store.svelte";

describe("KonvaStage", () => {
  beforeEach(() => {
    stageEvents.clear();
    annotationStore.reset();
  });
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

  it("resizes the Konva canvas when the native viewport changes", async () => {
    stageSize.mockClear();
    const props = {
      assetUrl: "data:image/png;base64,AAAA",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 2560, height: 1440 } },
      stageWidth: 1280,
      stageHeight: 720,
      onSelectionChange: () => {},
    };
    const { rerender } = render(KonvaStage, { props });
    await rerender({ ...props, stageWidth: 1600, stageHeight: 900 });
    expect(stageSize).toHaveBeenLastCalledWith({ width: 1600, height: 900 });
  });

  it.each([false, true])(
    "finishes a real crop gesture with physical bounds and edit modifier %s",
    (ctrlKey) => {
      const onSelectionComplete = vi.fn();
      render(KonvaStage, {
        assetUrl: "data:image/png;base64,AAAA",
        bounds: { origin: { x: -100, y: 20 }, size: { width: 200, height: 200 } },
        stageWidth: 100,
        stageHeight: 100,
        onSelectionChange: vi.fn(),
        onSelectionComplete,
      });
      Object.assign(pointer, { x: 10, y: 10 });
      stageEvents.get("mousedown")!({ target: fakeImage.current, evt: {} });
      Object.assign(pointer, { x: 80, y: 60 });
      stageEvents.get("mousemove")!({ evt: {} });
      expect(onSelectionComplete).not.toHaveBeenCalled();
      stageEvents.get("mouseup")!({ evt: { ctrlKey, metaKey: false } });
      expect(onSelectionComplete).toHaveBeenCalledWith(
        { origin: { x: -80, y: 40 }, size: { width: 140, height: 100 } },
        ctrlKey,
      );
    },
  );
});
