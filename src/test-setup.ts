// Global Vitest setup. Loaded before every test file.
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, vi } from "vitest";

// jsdom does not implement HTMLCanvasElement.getContext. Konva calls it
// during construction so we provide a no-op stub that lets the rest of
// the renderer mount.
if (typeof HTMLCanvasElement !== "undefined") {
  HTMLCanvasElement.prototype.getContext = vi.fn(
    () => null,
  ) as unknown as typeof HTMLCanvasElement.prototype.getContext;
}

// Capture console output during tests so we can spot unexpected errors.
const originalError = console.error;
beforeEach(() => {
  console.error = (...args: unknown[]) => {
    if (typeof args[0] === "string" && args[0].includes("not wrapped in act(")) {
      return;
    }
    originalError(...args);
  };
});
afterEach(() => {
  console.error = originalError;
});
