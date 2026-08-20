// Behavioural tests for the pin store. The store is the front-end half
// of the pin IPC: it round-trips every command through the typed Tauri
// wrapper and reflects the returned view model into the rune state.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as commands from "./commands";
import { pinStore } from "./pinStore.svelte";
import type { PinViewModel } from "./types";

function makeView(id: string, opacity = 1.0, x = 0): PinViewModel {
  return {
    id,
    transform: {
      position: { x, y: 0 },
      windowSize: { width: 200, height: 100 },
      sourceSize: { width: 200, height: 100 },
      zoom: 1.0,
      opacity,
    },
    source: {
      captureId: id,
      pngPath: `/cache/${id}.png`,
      bounds: { origin: { x: 0, y: 0 }, size: { width: 200, height: 100 } },
    },
  };
}

describe("pinStore", () => {
  beforeEach(() => {
    pinStore.resetForTesting();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("openPin reflects the new view model", async () => {
    vi.spyOn(commands, "openPin").mockResolvedValue({
      status: "ok",
      data: makeView("p-1"),
    });
    const view = await pinStore.openPin({
      captureId: "p-1",
      pngPath: "/cache/p-1.png",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 200, height: 100 } },
    });
    expect(view?.id).toBe("p-1");
    expect(pinStore.current.pins).toHaveLength(1);
  });

  it("closePin removes the pin from the store", async () => {
    vi.spyOn(commands, "openPin").mockResolvedValue({
      status: "ok",
      data: makeView("p-1"),
    });
    vi.spyOn(commands, "closePin").mockResolvedValue({ status: "ok", data: null });
    const view = await pinStore.openPin({
      captureId: "p-1",
      pngPath: "/cache/p-1.png",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 200, height: 100 } },
    });
    expect(view).not.toBeNull();
    await pinStore.closePin("p-1");
    expect(pinStore.current.pins).toHaveLength(0);
  });

  it("applyCommand uploads the latest view model", async () => {
    vi.spyOn(commands, "openPin").mockResolvedValue({
      status: "ok",
      data: makeView("p-1"),
    });
    vi.spyOn(commands, "applyPinCommand").mockResolvedValue({
      status: "ok",
      data: makeView("p-1", 0.5),
    });
    await pinStore.openPin({
      captureId: "p-1",
      pngPath: "/cache/p-1.png",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 200, height: 100 } },
    });
    await pinStore.applyCommand("p-1", { kind: "setOpacity", opacity: 0.5 });
    expect(pinStore.current.pins[0].transform.opacity).toBe(0.5);
  });

  it("records the error from a failed IPC", async () => {
    vi.spyOn(commands, "openPin").mockResolvedValue({
      status: "err",
      error: { kind: "invalid_payload", message: "bad" },
    });
    const view = await pinStore.openPin({
      captureId: "p-x",
      pngPath: "/cache/x.png",
      bounds: { origin: { x: 0, y: 0 }, size: { width: 10, height: 10 } },
    });
    expect(view).toBeNull();
    expect(pinStore.current.lastError).toBe("bad");
  });
});
