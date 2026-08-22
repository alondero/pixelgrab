import { describe, expect, it } from "vitest";
import { commitOptions } from "./commitIntent";

describe("overlay commit destinations", () => {
  it("publishes Enter to the shelf and clipboard", () => {
    expect(commitOptions("shelf")).toEqual({ toShelf: true, toClipboard: true });
  });

  it("keeps Ctrl+C clipboard-only", () => {
    expect(commitOptions("clipboard")).toEqual({ toShelf: false, toClipboard: true });
  });
});
