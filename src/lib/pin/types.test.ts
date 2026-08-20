// Contract-mirror tests for the pin wire shapes. The Rust-side fields
// are checked in `src-tauri/tests/ipc_contracts.rs`; this file mirrors
// the same field names on the TypeScript side so a rename on one side
// without a matching rename on the other fails the test.

import { describe, expect, it } from "vitest";

import { PIN_LIMITS, type PinCommand, type PinViewModel } from "./types";

describe("pin wire shape", () => {
  it("exposes the documented zoom and opacity bounds", () => {
    expect(PIN_LIMITS.minZoom).toBe(0.2);
    expect(PIN_LIMITS.maxZoom).toBe(4.0);
    expect(PIN_LIMITS.minOpacity).toBe(0.2);
    expect(PIN_LIMITS.maxOpacity).toBe(1.0);
    expect(PIN_LIMITS.defaultZoom).toBe(1.0);
    expect(PIN_LIMITS.defaultOpacity).toBe(1.0);
  });

  it("uses camelCase for every region field", () => {
    const view: PinViewModel = {
      id: "p-1",
      transform: {
        position: { x: 0, y: 0 },
        windowSize: { width: 200, height: 100 },
        sourceSize: { width: 200, height: 100 },
        zoom: 1.0,
        opacity: 1.0,
      },
      source: {
        captureId: "c-1",
        pngPath: "/cache/c-1.png",
        bounds: { origin: { x: 0, y: 0 }, size: { width: 200, height: 100 } },
      },
    };
    expect(view.transform.windowSize).toBeDefined();
    expect(view.transform.sourceSize).toBeDefined();
    expect(view.source.captureId).toBeDefined();
    expect(view.source.pngPath).toBeDefined();
  });

  it("supports the documented command variants", () => {
    const drag: PinCommand = { kind: "drag", dx: 10, dy: 20 };
    const zoom: PinCommand = { kind: "zoom", factor: 1.1, cursorX: 50, cursorY: 25 };
    const opacity: PinCommand = { kind: "setOpacity", opacity: 0.5 };
    const reset: PinCommand = { kind: "reset" };
    expect([drag, zoom, opacity, reset]).toHaveLength(4);
  });
});
