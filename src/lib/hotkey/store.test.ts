// Tests for the hotkey store svelte module. Pin the
// canonicaliseChord grammar + the action-binding helpers so the
// Rust core and the Svelte UI cannot drift apart silently.

import { describe, expect, it } from "vitest";
import type { HotkeyBindingsDto } from "$lib/ipc/types";
import modifierAliases from "$contracts/data/hotkey_modifiers.json";
import {
  actionBinding,
  canonicaliseChord,
  HOTKEY_ACTIONS,
  HOTKEY_LABELS,
  setActionBinding,
} from "./store.svelte";

const SAMPLE: HotkeyBindingsDto = {
  schemaVersion: 1,
  regionCapture: "CommandOrControl+Shift+S",
  fullScreenCapture: "CommandOrControl+Shift+F",
  shelfToggle: "CommandOrControl+Shift+L",
  paused: false,
};

describe("hotkey store helpers", () => {
  it("lists the three canonical actions in order", () => {
    expect(HOTKEY_ACTIONS).toEqual(["region_capture", "full_screen_capture", "shelf_toggle"]);
  });

  it("exposes a label for every action", () => {
    for (const action of HOTKEY_ACTIONS) {
      expect(HOTKEY_LABELS[action]).toBeTruthy();
    }
  });

  it("reads and writes a single action binding", () => {
    expect(actionBinding(SAMPLE, "region_capture")).toBe("CommandOrControl+Shift+S");
    const next = setActionBinding(SAMPLE, "full_screen_capture", "F12");
    expect(actionBinding(next, "full_screen_capture")).toBe("F12");
    expect(actionBinding(next, "region_capture")).toBe("CommandOrControl+Shift+S");
  });

  it("treats an unset binding as undefined", () => {
    const empty: HotkeyBindingsDto = { schemaVersion: 1 };
    expect(actionBinding(empty, "shelf_toggle")).toBeUndefined();
  });

  it("clears an action binding when the new value is null", () => {
    const cleared = setActionBinding(SAMPLE, "shelf_toggle", null);
    expect(actionBinding(cleared, "shelf_toggle")).toBeUndefined();
    expect(actionBinding(cleared, "region_capture")).toBe("CommandOrControl+Shift+S");
  });
});

describe("canonicaliseChord", () => {
  it("canonicalises modifier aliases", () => {
    expect(canonicaliseChord(["Ctrl", "S"])).toBe("CommandOrControl+S");
    expect(canonicaliseChord(["cmd", "shift", "s"])).toBe("CommandOrControl+Shift+S");
    expect(canonicaliseChord(["alt", "f4"])).toBe("Alt+F4");
    // Meta + Win both map to the same canonical modifier, so
    // the chord collapses to a single Super + main key — bare
    // Super without a main key is rejected. (Tracer 14
    // follow-up: the canonical form is "Super", mirroring
    // `pixelgrab_contracts::hotkey::parse_binding`.)
    expect(canonicaliseChord(["Meta", "Win"])).toBeNull();
    expect(canonicaliseChord(["Meta", "Tab"])).toBe("Super+TAB");
  });

  it("sorts modifiers in canonical order", () => {
    expect(canonicaliseChord(["Shift", "Ctrl", "Alt", "S"])).toBe("CommandOrControl+Alt+Shift+S");
  });

  it("rejects bare main keys without a modifier", () => {
    expect(canonicaliseChord(["S"])).toBeNull();
  });

  it("accepts function keys without a modifier", () => {
    expect(canonicaliseChord(["F12"])).toBe("F12");
  });

  it("rejects two main keys", () => {
    expect(canonicaliseChord(["Ctrl", "A", "B"])).toBeNull();
  });

  it("rejects empty chord", () => {
    expect(canonicaliseChord([])).toBeNull();
  });

  it("deduplicates repeated modifiers", () => {
    expect(canonicaliseChord(["Ctrl", "Control", "S"])).toBe("CommandOrControl+S");
  });
});

/// Tracer 14 follow-up (issue #46): every alias in the shared
/// `hotkey_modifiers.json` must round-trip through the
/// canonicaliser. This is the "TS + Rust modifier aliases
/// round-trip via a single test" acceptance criterion: both
/// sides import the same file, so iterating it here pins the
/// Rust parser's behaviour from the JS test surface (and vice
/// versa).
describe("modifier alias round-trip via shared JSON", () => {
  type ModifierAliasTable = {
    modifiers: Array<{ canonical: string; aliases: string[] }>;
    rank: string[];
  };

  it("resolves every alias in the shared JSON to its canonical name", () => {
    const table = modifierAliases as ModifierAliasTable;
    expect(table.modifiers.length).toBeGreaterThan(0);
    for (const entry of table.modifiers) {
      for (const alias of entry.aliases) {
        const chord = canonicaliseChord([alias, "S"]);
        // `toMatch` only takes the expected pattern; surface the
        // offending alias + canonical in the assertion message
        // so a failure points the developer at the JSON entry
        // that drifted out of sync.
        expect(
          chord,
          `alias ${alias} must canonicalise to ${entry.canonical}, got ${chord}`,
        ).toMatch(new RegExp(`^${entry.canonical}\\+S$`));
      }
    }
  });

  it("respects the rank order declared in the shared JSON", () => {
    const table = modifierAliases as ModifierAliasTable;
    // Pick the canonical name of every modifier; submit them in
    // reverse order; expect the canonicaliser to re-sort them
    // into the order declared in the JSON's `rank` array.
    const canonicals = table.modifiers.map((m) => m.canonical);
    const reverseInput = [...canonicals].reverse().concat(["S"]);
    const chord = canonicaliseChord(reverseInput);
    expect(chord).not.toBeNull();
    const parts = chord!.split("+");
    const expected = [...table.rank, "S"];
    expect(parts).toEqual(expected);
  });
});
