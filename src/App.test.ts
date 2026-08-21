// Mount the main App against the mock IPC layer and verify the
// tray-intent -> capture -> cancel flow renders the expected UI.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";

// Mock the IPC commands module so the component pulls in the deterministic
// shell implementation instead of the Tauri runtime.
vi.mock("$lib/ipc/commands", async () => {
  const shell = await import("$lib/ipc/shell.svelte");
  return {
    requestCapture: shell.mockRequestCapture,
    requestCommit: shell.mockRequestCommit,
    requestCancel: shell.mockRequestCancel,
    getSessionSnapshot: shell.mockGetSessionSnapshot,
    getShelfPreferences: shell.mockGetShelfPreferences,
    updateShelfPreferences: shell.mockUpdateShelfPreferences,
    getHotkeyBindings: shell.mockGetHotkeyBindings,
    updateHotkeyBindings: shell.mockUpdateHotkeyBindings,
    getHotkeyStatus: shell.mockGetHotkeyStatus,
    setHotkeyPaused: shell.mockSetHotkeyPaused,
  };
});

// Mock the @tauri-apps/api/event module so the App's onMount doesn't throw.
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

import App from "./App.svelte";
import { __resetMock } from "$lib/ipc/shell.svelte";

describe("App", () => {
  beforeEach(() => {
    __resetMock();
  });

  it("renders the initial idle state", () => {
    render(App);
    expect(screen.getByTestId("session-state")).toHaveTextContent("idle");
  });

  it("updates the session state after a capture", async () => {
    const user = userEvent.setup();
    render(App);
    await user.click(screen.getByRole("button", { name: /trigger capture/i }));
    const state = await screen.findByTestId("session-state");
    expect(state).toHaveTextContent("ready");
    expect(screen.getByTestId("session-capture-id")).toBeTruthy();
  });

  it("exposes a cancel button that returns the session to idle", async () => {
    const user = userEvent.setup();
    render(App);
    await user.click(screen.getByRole("button", { name: /trigger capture/i }));
    expect(screen.getByTestId("session-state")).toHaveTextContent("ready");
    await user.click(screen.getByRole("button", { name: /cancel/i }));
    expect(screen.getByTestId("session-state")).toHaveTextContent("idle");
  });

  // Tracer 15 closes the documentation-vs-implementation gap from
  // `docs/ACCESSIBILITY.md`: every <button> in the main window
  // must have either visible text content or an aria-label so a
  // screen reader can announce a name for it.
  it("every button has either visible text or an aria-label", () => {
    const { container } = render(App);
    const buttons = container.querySelectorAll("button");
    expect(buttons.length).toBeGreaterThan(0);
    for (const button of buttons) {
      const text = (button.textContent ?? "").trim();
      const ariaLabel = button.getAttribute("aria-label")?.trim() ?? "";
      expect(text.length + ariaLabel.length).toBeGreaterThan(0);
    }
  });
});
