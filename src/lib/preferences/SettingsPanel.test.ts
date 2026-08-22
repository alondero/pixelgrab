// Smoke tests for the SettingsPanel component. We exercise the
// controlled-component contract (click → store.applyPatch) by
// substituting a fake store; the round-trip tests against the real
// Rust core live in `src-tauri/tests/shelf_preferences_integration.rs`.

import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import SettingsPanel from "./SettingsPanel.svelte";
import type { PreferencesStore } from "./store.svelte";
import type { ShelfPreferencesDto } from "$lib/ipc/types";

const SAMPLE: ShelfPreferencesDto = {
  schemaVersion: 1,
  corner: "bottom_right",
  targetMonitorId: null,
  marginPx: 24,
  autoDismissEnabled: true,
  lifetimeSeconds: 60,
  visibleCardCount: 4,
  showCountdown: true,
};

function fakeStore(value: ShelfPreferencesDto = SAMPLE): {
  store: PreferencesStore;
  applyPatch: ReturnType<typeof vi.fn>;
  commitPreferences: ReturnType<typeof vi.fn>;
} {
  const applyPatch = vi.fn(async () => {});
  const commitPreferences = vi.fn(async () => {});
  // The component reads `store.value`, `store.loading`, `store.error`,
  // and calls `store.refresh`, `store.applyPatch`, `store.commitPreferences`.
  const store = {
    get value() {
      return value;
    },
    get loading() {
      return false;
    },
    get error() {
      return null;
    },
    refresh: vi.fn(async () => {}),
    applyPatch,
    commitPreferences,
  } as unknown as PreferencesStore;
  return { store, applyPatch, commitPreferences };
}

describe("SettingsPanel", () => {
  it("renders all four corner buttons", () => {
    const { store } = fakeStore();
    render(SettingsPanel, { store });
    expect(screen.getByTestId("corner-top_left")).toBeTruthy();
    expect(screen.getByTestId("corner-top_right")).toBeTruthy();
    expect(screen.getByTestId("corner-bottom_left")).toBeTruthy();
    expect(screen.getByTestId("corner-bottom_right")).toBeTruthy();
  });

  it("marks the current corner as pressed", () => {
    const { store } = fakeStore();
    render(SettingsPanel, { store });
    const br = screen.getByTestId("corner-bottom_right") as HTMLButtonElement;
    expect(br.getAttribute("aria-pressed")).toBe("true");
    const tl = screen.getByTestId("corner-top_left") as HTMLButtonElement;
    expect(tl.getAttribute("aria-pressed")).toBe("false");
  });

  it("clicking a corner patch fires applyPatch", async () => {
    const user = userEvent.setup();
    const { store, applyPatch } = fakeStore();
    render(SettingsPanel, { store });
    await user.click(screen.getByTestId("corner-top_left"));
    expect(applyPatch).toHaveBeenCalledWith({ corner: "top_left" });
  });

  it("Apply button calls commitPreferences", async () => {
    const user = userEvent.setup();
    const { store, commitPreferences } = fakeStore();
    render(SettingsPanel, { store });
    await user.click(screen.getByTestId("apply-button"));
    expect(commitPreferences).toHaveBeenCalled();
  });

  it("shows lifetime presets when auto-dismiss is enabled", () => {
    const { store } = fakeStore();
    render(SettingsPanel, { store });
    expect(screen.getByTestId("lifetime-preset-30")).toBeTruthy();
    expect(screen.getByTestId("lifetime-slider")).toBeTruthy();
  });

  it("hides lifetime presets when auto-dismiss is disabled", () => {
    const off = { ...SAMPLE, autoDismissEnabled: false };
    const { store } = fakeStore(off);
    render(SettingsPanel, { store });
    expect(screen.queryByTestId("lifetime-preset-30")).toBeNull();
  });

  it("clicking a lifetime preset patch fires applyPatch", async () => {
    const user = userEvent.setup();
    const { store, applyPatch } = fakeStore();
    render(SettingsPanel, { store });
    await user.click(screen.getByTestId("lifetime-preset-30"));
    expect(applyPatch).toHaveBeenCalledWith({ lifetimeSeconds: 30 });
  });

  it("renders margin, visible-card, and countdown controls", () => {
    const { store } = fakeStore();
    render(SettingsPanel, { store });
    expect(screen.getByTestId("margin-slider")).toBeTruthy();
    expect(screen.getByTestId("visible-card-stepper")).toBeTruthy();
    expect(screen.getByTestId("countdown-toggle")).toBeTruthy();
    expect(screen.getByTestId("auto-dismiss-toggle")).toBeTruthy();
  });

  // Issue #63: cursor-monitor targeting.
  it("offers a cursor-monitor option in the display picker", () => {
    const { store } = fakeStore();
    render(SettingsPanel, { store });
    const option = screen.getByTestId("display-option-cursor") as HTMLOptionElement;
    expect(option.value).toBe("cursor");
  });

  it("selecting the cursor option patches targetMonitorId to 'cursor'", async () => {
    const user = userEvent.setup();
    const { store, applyPatch } = fakeStore();
    render(SettingsPanel, { store });
    await user.selectOptions(screen.getByTestId("display-picker"), "cursor");
    expect(applyPatch).toHaveBeenCalledWith({ targetMonitorId: "cursor" });
  });

  // Issue #63: live placement preview.
  it("renders a placement preview whose shelf rect sits at the chosen corner", () => {
    const { store } = fakeStore();
    render(SettingsPanel, { store });
    const preview = screen.getByTestId("placement-preview");
    expect(preview).toBeTruthy();
    const rect = screen.getByTestId("placement-rect");
    expect(rect.getAttribute("data-corner")).toBe("bottom_right");
    // Margin is reflected on the preview so the user sees the gap.
    expect(rect.getAttribute("data-margin")).toBe("24");
  });

  it("moves the preview shelf rect when the corner changes in prefs", () => {
    const { store } = fakeStore({ ...SAMPLE, corner: "top_left" });
    render(SettingsPanel, { store });
    expect(screen.getByTestId("placement-rect").getAttribute("data-corner")).toBe("top_left");
  });
});
