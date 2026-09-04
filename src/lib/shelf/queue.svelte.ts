// Per-card countdown state. The Rust core owns the authoritative
// timer state and pushes updates via the `pixelgrab://shelf-queue-updated`
// event; this module drives the visual countdown using `performance.now()`
// and `requestAnimationFrame` so the UI never waits for a Rust round-trip
// to redraw the seconds-remaining text.
//
// The frontend is not the sole expiry driver: the Rust core spawns a
// background ticker in `pixelgrab_lib::spawn_shelf_ticker` that runs
// `queue.tick` on its own clock so cards expire even when the webview
// is hidden or throttled. The rAF loop here is purely cosmetic.

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
 * A clock store that exposes the current elapsed millis as a
 * Svelte-reactive getter, driven by `requestAnimationFrame` so the
 * countdown text re-renders smoothly without backend round-trips.
 *
 * The `start()` and `stop()` methods are idempotent; `stop()`
 * cancels the rAF handle so the loop does not leak when the queue
 * empties.
 */
export function createClockStore(): {
  readonly nowMs: number;
  readNowMs(): number;
  sync(authoritativeNowMs: number): void;
  start(): void;
  stop(): void;
} {
  let localAnchorMs = nowElapsedMs();
  let authoritativeAnchorMs = 0;
  let nowMs = $state(0);
  let rafId = 0;
  let running = false;

  function tick() {
    nowMs = authoritativeAnchorMs + (nowElapsedMs() - localAnchorMs);
    if (running && typeof requestAnimationFrame === "function") {
      rafId = requestAnimationFrame(tick);
    }
  }

  return {
    get nowMs() {
      return nowMs;
    },
    readNowMs() {
      // This accessor intentionally does not read the reactive `nowMs` state.
      // Expiry checks can sample the authoritative interpolated clock without
      // subscribing themselves to the 60/144 Hz render ticker.
      return authoritativeAnchorMs + (nowElapsedMs() - localAnchorMs);
    },
    sync(authoritativeNowMs: number) {
      localAnchorMs = nowElapsedMs();
      authoritativeAnchorMs = authoritativeNowMs;
      nowMs = authoritativeNowMs;
    },
    start() {
      if (running) return;
      running = true;
      if (typeof requestAnimationFrame === "function") {
        rafId = requestAnimationFrame(tick);
      }
    },
    stop() {
      if (!running) return;
      running = false;
      if (rafId !== 0 && typeof cancelAnimationFrame === "function") {
        cancelAnimationFrame(rafId);
      }
      rafId = 0;
    },
  };
}
