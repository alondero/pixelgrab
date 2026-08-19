// Tests for the preferences store's sanitize helper and the
// preference DTO shape. The store itself runs against the real
// Tauri IPC, so its end-to-end behaviour is exercised by the Rust
// integration tests in `src-tauri/tests/shelf_preferences_integration.rs`.

import { describe, expect, it } from "vitest";
import { sanitizeClient } from "./store.svelte";
import {
  MAX_LIFETIME_SECONDS,
  MAX_MARGIN_PX,
  MAX_VISIBLE_CARDS,
  MIN_LIFETIME_SECONDS,
  MIN_MARGIN_PX,
  MIN_VISIBLE_CARDS,
} from "./constants";
import type { ShelfPreferencesDto } from "$lib/ipc/types";

const BASE: ShelfPreferencesDto = {
  schemaVersion: 1,
  corner: "bottom_right",
  targetMonitorId: null,
  marginPx: 24,
  autoDismissEnabled: true,
  lifetimeSeconds: 60,
  visibleCardCount: 4,
  showCountdown: true,
};

describe("sanitizeClient", () => {
  it("returns defaults for the canonical input", () => {
    const out = sanitizeClient(BASE);
    expect(out.corner).toBe("bottom_right");
    expect(out.marginPx).toBe(24);
    expect(out.lifetimeSeconds).toBe(60);
    expect(out.visibleCardCount).toBe(4);
  });

  it("clamps oversized margins down to MAX_MARGIN_PX", () => {
    const out = sanitizeClient({ ...BASE, marginPx: 9999 });
    expect(out.marginPx).toBe(MAX_MARGIN_PX);
  });

  it("clamps negative margins up to MIN_MARGIN_PX", () => {
    const out = sanitizeClient({ ...BASE, marginPx: -50 });
    expect(out.marginPx).toBe(MIN_MARGIN_PX);
  });

  it("clamps lifetimes to documented seconds range", () => {
    const tooShort = sanitizeClient({ ...BASE, lifetimeSeconds: 1 });
    expect(tooShort.lifetimeSeconds).toBe(MIN_LIFETIME_SECONDS);
    const tooLong = sanitizeClient({ ...BASE, lifetimeSeconds: 999_999 });
    expect(tooLong.lifetimeSeconds).toBe(MAX_LIFETIME_SECONDS);
  });

  it("clamps visible-card count to documented range", () => {
    const tooSmall = sanitizeClient({ ...BASE, visibleCardCount: 0 });
    expect(tooSmall.visibleCardCount).toBe(MIN_VISIBLE_CARDS);
    const tooLarge = sanitizeClient({ ...BASE, visibleCardCount: 99 });
    expect(tooLarge.visibleCardCount).toBe(MAX_VISIBLE_CARDS);
  });

  it("treats NaN as the minimum", () => {
    const out = sanitizeClient({ ...BASE, marginPx: Number.NaN });
    expect(out.marginPx).toBe(MIN_MARGIN_PX);
  });

  it("coerces booleans from truthy/falsy values", () => {
    const on = sanitizeClient({ ...BASE, autoDismissEnabled: 1 as unknown as boolean });
    expect(on.autoDismissEnabled).toBe(true);
    const off = sanitizeClient({ ...BASE, autoDismissEnabled: 0 as unknown as boolean });
    expect(off.autoDismissEnabled).toBe(false);
  });

  it("passes the corner enum through unchanged", () => {
    for (const corner of ["top_left", "top_right", "bottom_left", "bottom_right"] as const) {
      const out = sanitizeClient({ ...BASE, corner });
      expect(out.corner).toBe(corner);
    }
  });
});
