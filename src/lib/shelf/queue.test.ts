import { describe, expect, it, vi } from "vitest";
import {
  createClockStore,
  DEFAULT_TIMER_CONFIG,
  formatRemaining,
  remainingMs,
} from "./queue.svelte";
import type { ShelfTimerState } from "$lib/ipc/types";

describe("queue.svelte", () => {
  it("exposes the documented default timer config", () => {
    expect(DEFAULT_TIMER_CONFIG.lifetimeMs).toBe(60_000);
    expect(DEFAULT_TIMER_CONFIG.graceMs).toBe(3_000);
  });

  it("remainingMs returns deadline minus now when running", () => {
    const timer: ShelfTimerState = {
      addedAtElapsedMs: 0,
      deadlineAtElapsedMs: 10_000,
    };
    expect(remainingMs(timer, 2_500)).toBe(7_500);
    expect(remainingMs(timer, 12_000)).toBe(0);
  });

  it("remainingMs returns the captured value while paused", () => {
    const timer: ShelfTimerState = {
      addedAtElapsedMs: 0,
      deadlineAtElapsedMs: 60_000,
      pausedAtElapsedMs: 1_000,
      pausedRemainingMs: 4_000,
    };
    // Even at now = 999 999 (way past the deadline), the captured
    // paused value wins so the countdown stops advancing visually.
    expect(remainingMs(timer, 999_999)).toBe(4_000);
  });

  it("formatRemaining rounds to whole seconds", () => {
    expect(formatRemaining(0)).toBe("expired");
    expect(formatRemaining(1)).toBe("0s");
    expect(formatRemaining(500)).toBe("1s");
    expect(formatRemaining(1_499)).toBe("1s");
    expect(formatRemaining(1_500)).toBe("2s");
    expect(formatRemaining(60_000)).toBe("60s");
  });

  it("anchors the browser clock to the backend snapshot epoch", () => {
    const now = vi.spyOn(performance, "now");
    now.mockReturnValue(25_000);
    const clock = createClockStore();
    clock.sync(4_000);
    expect(clock.nowMs).toBe(4_000);
    expect(clock.readNowMs()).toBe(4_000);
    now.mockRestore();
  });
});
