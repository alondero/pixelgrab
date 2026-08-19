// Per-card countdown state. The Rust core owns the authoritative
// timer state and pushes updates via the `pixelgrab://shelf-queue-updated`
// event; this module drives the visual countdown using `performance.now()`
// and `requestAnimationFrame` so the UI never waits for a Rust round-trip
// to redraw the seconds-remaining text.
//
// The frontend does NOT push expiry events to the backend; the backend
// has its own tick (the `tick_shelf_queue` IPC plus a periodic check on
// commit/dismiss). The visual countdown just fades cards out smoothly
// at the moment their deadline elapses — the authoritative dismissal
// happens server-side.

import type { ShelfTimerConfig, ShelfTimerState } from "$lib/ipc/types";

/**
 * The timer configuration the frontend uses to render countdowns.
 * Mirrors the Rust `ShelfTimerConfig`. Defaults match the contract
 * defaults (60 s lifetime, 3 s grace).
 */
export const DEFAULT_TIMER_CONFIG: ShelfTimerConfig = {
  lifetimeMs: 60_000,
  graceMs: 3_000,
};

/**
 * Read the front-end "now" monotonic millis. Falls back to `Date.now()`
 * when `performance` is not available (e.g. in older test environments).
 */
export function nowElapsedMs(): number {
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return performance.now();
  }
  return Date.now();
}

/**
 * Compute the remaining time for a card at `now_ms`. Mirrors the
 * Rust `ShelfTimerState::remaining_ms` policy so the visual countdown
 * matches the authoritative server state.
 *
 * - While paused, the captured `pausedRemainingMs` is returned.
 * - While running, `deadlineAtElapsedMs - now_ms` (clamped at zero).
 */
export function remainingMs(timer: ShelfTimerState, nowMs: number): number {
  if (timer.pausedAtElapsedMs !== undefined) {
    return timer.pausedRemainingMs ?? 0;
  }
  return Math.max(0, timer.deadlineAtElapsedMs - nowMs);
}

/**
 * Format a remaining-time millis value as a short string suitable for
 * the shelf card's metadata line. Returns "60s" through "0s" with no
 * fractional digits, and "expired" for non-positive values.
 */
export function formatRemaining(ms: number): string {
  if (ms <= 0) return "expired";
  const seconds = Math.max(0, Math.round(ms / 1000));
  return `${seconds}s`;
}

/**
 * Build a Svelte-reactive object holding the current elapsed-ms clock.
 * The queue component subscribes to `nowElapsedMs` and re-renders each
 * card's countdown text on every animation frame.
 */
export function createClockStore(): { nowMs: number } {
  let nowMs = $state(nowElapsedMs());
  if (typeof requestAnimationFrame === "function") {
    let raf = 0;
    const tick = () => {
      nowMs = nowElapsedMs();
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    // No teardown yet — the shelf window lives for the lifetime of the
    // process. A future tracer can introduce a `destroy()` hook.
    void raf;
  }
  return {
    get nowMs() {
      return nowMs;
    },
  };
}
