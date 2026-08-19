// Svelte store for the shelf preferences. Holds the live in-memory
// state and proxies writes through the IPC so the Rust core
// schedules the debounced disk write. Reads also go through the IPC
// on mount so the initial state matches what the Rust core loaded
// from disk.
//
// The store is intentionally tiny — it's a thin reactive wrapper
// around `get_shelf_preferences` / `update_shelf_preferences`. The
// Rust core owns the canonical state; this module just makes the
// shape reactive.

import { getShelfPreferences, updateShelfPreferences } from "$lib/ipc/commands";
import type { ShelfPreferencesDto } from "$lib/ipc/types";
import {
  MAX_LIFETIME_SECONDS,
  MAX_MARGIN_PX,
  MAX_VISIBLE_CARDS,
  MIN_LIFETIME_SECONDS,
  MIN_MARGIN_PX,
  MIN_VISIBLE_CARDS,
} from "./constants";

// Default preferences used when the backend fails to load. Mirrors
// `ShelfPreferences::default()` on the Rust side so a startup race
// (IPC not ready yet) renders the controls in their expected state.
const DEFAULT_PREFS: ShelfPreferencesDto = {
  schemaVersion: 1,
  corner: "bottom_right",
  targetMonitorId: null,
  marginPx: 24,
  autoDismissEnabled: true,
  lifetimeSeconds: 60,
  visibleCardCount: 4,
  showCountdown: true,
};

function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.max(min, Math.min(max, value));
}

export function sanitizeClient(prefs: ShelfPreferencesDto): ShelfPreferencesDto {
  return {
    schemaVersion: 1,
    corner: prefs.corner,
    targetMonitorId: prefs.targetMonitorId ?? null,
    marginPx: clamp(prefs.marginPx, MIN_MARGIN_PX, MAX_MARGIN_PX),
    autoDismissEnabled: !!prefs.autoDismissEnabled,
    lifetimeSeconds: clamp(prefs.lifetimeSeconds, MIN_LIFETIME_SECONDS, MAX_LIFETIME_SECONDS),
    visibleCardCount: clamp(prefs.visibleCardCount, MIN_VISIBLE_CARDS, MAX_VISIBLE_CARDS),
    showCountdown: !!prefs.showCountdown,
  };
}

export function createPreferencesStore() {
  let current = $state<ShelfPreferencesDto>({ ...DEFAULT_PREFS });
  let loading = $state(false);
  let lastError = $state<string | null>(null);

  async function refresh(): Promise<void> {
    loading = true;
    lastError = null;
    const response = await getShelfPreferences();
    if (response.status === "ok") {
      current = response.data;
    } else {
      lastError = response.error.message;
    }
    loading = false;
  }

  // Apply a partial patch. The patch is sanitized client-side so the
  // sliders cannot exceed their clamps, then sent to the Rust core
  // with `commit = false` for live preview. The Rust core mirrors the
  // in-memory state but only persists after `commitPreferences`.
  async function applyPatch(patch: Partial<ShelfPreferencesDto>): Promise<void> {
    const next = sanitizeClient({ ...current, ...patch });
    current = next;
    const response = await updateShelfPreferences({
      preferences: next,
      commit: false,
    });
    if (response.status === "err") {
      lastError = response.error.message;
    }
  }

  // Commit the current preferences: persist to disk, apply timer
  // config to the running shelf, and re-emit the queue snapshot so
  // the shelf window repositions itself immediately.
  async function commitPreferences(): Promise<void> {
    const sanitized = sanitizeClient(current);
    current = sanitized;
    const response = await updateShelfPreferences({
      preferences: sanitized,
      commit: true,
    });
    if (response.status === "ok") {
      current = response.data;
    } else {
      lastError = response.error.message;
    }
  }

  return {
    get value() {
      return current;
    },
    get loading() {
      return loading;
    },
    get error() {
      return lastError;
    },
    refresh,
    applyPatch,
    commitPreferences,
  };
}

export type PreferencesStore = ReturnType<typeof createPreferencesStore>;
