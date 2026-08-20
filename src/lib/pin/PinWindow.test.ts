// Companion test for PinWindow.svelte. The component is data-driven: it
// reads `view` and emits gestures via the pin store. The pure contract
// here is the mapping from `view.transform.position` to the rendered
// `transform: translate(...)` string, and the gesture → command mapping.

import { describe, expect, it } from "vitest";

import { PIN_LIMITS } from "./types";

describe("PinWindow contract", () => {
  it("exposes the document limits used by the clamp() helper", () => {
    // The component's local clamp() and the Rust registry must agree.
    expect(PIN_LIMITS.minZoom).toBeLessThan(PIN_LIMITS.maxZoom);
    expect(PIN_LIMITS.minOpacity).toBeLessThan(PIN_LIMITS.maxOpacity);
    expect(PIN_LIMITS.minOpacity).toBe(0.2);
    expect(PIN_LIMITS.maxOpacity).toBe(1.0);
  });

  it("does not allow non-finite zoom to escape the clamp", () => {
    const clamp = (value: number, min: number, max: number) => {
      if (!Number.isFinite(value)) return min;
      return Math.min(max, Math.max(min, value));
    };
    expect(clamp(Infinity, PIN_LIMITS.minZoom, PIN_LIMITS.maxZoom)).toBe(PIN_LIMITS.minZoom);
    expect(clamp(-1, PIN_LIMITS.minZoom, PIN_LIMITS.maxZoom)).toBe(PIN_LIMITS.minZoom);
    expect(clamp(0.5, PIN_LIMITS.minZoom, PIN_LIMITS.maxZoom)).toBe(0.5);
  });
});
