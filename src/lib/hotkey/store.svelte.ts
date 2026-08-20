// Svelte store for the hotkey bindings. Holds the live in-memory
// state and proxies writes through the IPC so the Rust core
// persists atomically. The frontend never owns the canonical
// state — the registry has the OS handles.
//
// Mirrors `src/lib/preferences/store.svelte.ts` so a future
// "settings as one panel" refactor stays a single import swap.

import {
  getHotkeyBindings,
  getHotkeyStatus,
  setHotkeyPaused,
  updateHotkeyBindings,
} from "$lib/ipc/commands";
import type { HotkeyBindingsDto, HotkeyRegistryStatusDto } from "$lib/ipc/types";

// Tracer 14 follow-up (issue #46): modifier aliases are owned by
// `pixelgrab-contracts` and re-used here so the Svelte
// canonicaliser cannot drift from the Rust parser. The Rust side
// `include_str!`'s the same file from
// `crates/pixelgrab-contracts/data/hotkey_modifiers.json`; the
// Vite `$contracts` alias maps to the same directory so the JSON
// import resolves to a single file on disk.
import modifierAliases from "$contracts/data/hotkey_modifiers.json";

// Default bindings used when the IPC fails to load. Mirrors
// `HotkeyBindings::defaults()` on the Rust side.
const DEFAULT_BINDINGS: HotkeyBindingsDto = {
  schemaVersion: 1,
  regionCapture: "CommandOrControl+Shift+S",
  fullScreenCapture: "CommandOrControl+Shift+F",
  shelfToggle: "CommandOrControl+Shift+L",
  paused: false,
};

export type HotkeyAction = "region_capture" | "full_screen_capture" | "shelf_toggle";

export const HOTKEY_ACTIONS: HotkeyAction[] = [
  "region_capture",
  "full_screen_capture",
  "shelf_toggle",
];

export const HOTKEY_LABELS: Record<HotkeyAction, string> = {
  region_capture: "Capture Region",
  full_screen_capture: "Capture Full Screen",
  shelf_toggle: "Toggle Shelf",
};

// Shape of the shared JSON file. `rank` defines the canonical
// modifier order — must stay in lock-step with the Rust side.
type ModifierAliasTable = {
  modifiers: Array<{ canonical: string; aliases: string[] }>;
  rank: string[];
};

const aliasTable = modifierAliases as ModifierAliasTable;

// Reverse-lookup map: upper-cased alias → canonical name. Built
// once at module load; the JSON import is resolved by Vite at
// parse time so the table is stable for the lifetime of the
// bundle. Reused on every `canonicaliseChord` call without
// re-allocating.
const ALIAS_TO_CANONICAL = new Map<string, string>();
for (const entry of aliasTable.modifiers) {
  for (const alias of entry.aliases) {
    ALIAS_TO_CANONICAL.set(alias.toUpperCase(), entry.canonical);
  }
  ALIAS_TO_CANONICAL.set(entry.canonical.toUpperCase(), entry.canonical);
}

const RANK_INDEX = new Map<string, number>(aliasTable.rank.map((m, idx) => [m.toUpperCase(), idx]));

export function actionBinding(
  bindings: HotkeyBindingsDto,
  action: HotkeyAction,
): string | undefined {
  switch (action) {
    case "region_capture":
      return bindings.regionCapture ?? undefined;
    case "full_screen_capture":
      return bindings.fullScreenCapture ?? undefined;
    case "shelf_toggle":
      return bindings.shelfToggle ?? undefined;
  }
}

export function setActionBinding(
  bindings: HotkeyBindingsDto,
  action: HotkeyAction,
  value: string | null,
): HotkeyBindingsDto {
  const next: HotkeyBindingsDto = { ...bindings };
  switch (action) {
    case "region_capture":
      next.regionCapture = value;
      break;
    case "full_screen_capture":
      next.fullScreenCapture = value;
      break;
    case "shelf_toggle":
      next.shelfToggle = value;
      break;
  }
  return next;
}

// Canonicalise a user-pressed chord into the Rust-side grammar.
// Returns `null` when the chord has no main key (a bare modifier
// press is not a binding). The implementation is intentionally
// simple so the test suite can pin every accepted shape.
export function canonicaliseChord(parts: string[]): string | null {
  const modifiers: string[] = [];
  let main: string | null = null;
  for (const raw of parts) {
    const token = raw.toUpperCase();
    const canonical = ALIAS_TO_CANONICAL.get(token);
    if (canonical) {
      modifiers.push(canonical);
    } else if (!main) {
      main = token;
    } else {
      return null; // two main keys
    }
  }
  if (!main) return null;
  // Canonical modifier order — Ctrl, Alt, Shift, Meta (or
  // whatever the shared JSON's `rank` array says).
  modifiers.sort(
    (a, b) =>
      (RANK_INDEX.get(a.toUpperCase()) ?? Number.MAX_SAFE_INTEGER) -
      (RANK_INDEX.get(b.toUpperCase()) ?? Number.MAX_SAFE_INTEGER),
  );
  // Drop duplicate modifiers after normalisation.
  const unique = modifiers.filter((m, idx) => modifiers.indexOf(m) === idx);
  if (unique.length === 0) {
    // Allow function / nav keys without a modifier.
    if (/^F\d+$/.test(main) || /^(TAB|ENTER|ESCAPE|SPACE)$/.test(main)) {
      return main;
    }
    return null;
  }
  return [...unique, main].join("+");
}

export function createHotkeyStore() {
  let current = $state<HotkeyBindingsDto>({ ...DEFAULT_BINDINGS });
  let status = $state<HotkeyRegistryStatusDto>({ active: true, paused: false });
  let pendingError = $state<string | null>(null);
  let loading = $state(false);

  async function refresh(): Promise<void> {
    loading = true;
    pendingError = null;
    const [bindings, statusResponse] = await Promise.all([getHotkeyBindings(), getHotkeyStatus()]);
    if (bindings.status === "ok") {
      current = { ...DEFAULT_BINDINGS, ...bindings.data };
    } else {
      pendingError = bindings.error.message;
    }
    if (statusResponse.status === "ok") {
      status = statusResponse.data;
    } else if (!pendingError) {
      pendingError = statusResponse.error.message;
    }
    loading = false;
  }

  async function setBinding(action: HotkeyAction, value: string | null): Promise<void> {
    const next = setActionBinding(current, action, value);
    current = next;
    pendingError = null;
    const response = await updateHotkeyBindings({ bindings: next });
    if (response.status === "ok") {
      current = { ...DEFAULT_BINDINGS, ...response.data };
      // Refresh the status payload — the registry may have
      // changed `active` if the rebind removed the only binding.
      const statusResponse = await getHotkeyStatus();
      if (statusResponse.status === "ok") {
        status = statusResponse.data;
      }
    } else {
      pendingError = response.error.message;
      // Roll back the local copy so the UI mirrors the persisted
      // state on the next refresh.
      await refresh();
    }
  }

  async function togglePaused(): Promise<void> {
    pendingError = null;
    const nextPaused = !current.paused;
    current = { ...current, paused: nextPaused };
    const response = await setHotkeyPaused(nextPaused);
    if (response.status === "ok") {
      status = response.data;
    } else {
      pendingError = response.error.message;
      current = { ...current, paused: !nextPaused };
    }
  }

  return {
    get value() {
      return current;
    },
    get status() {
      return status;
    },
    get loading() {
      return loading;
    },
    get error() {
      return pendingError;
    },
    refresh,
    setBinding,
    togglePaused,
  };
}

export type HotkeyStore = ReturnType<typeof createHotkeyStore>;
