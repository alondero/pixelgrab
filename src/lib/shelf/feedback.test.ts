import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createFeedbackStore } from "./feedback.svelte";

describe("feedback store", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts with no message", () => {
    const store = createFeedbackStore();
    expect(store.message).toBeNull();
  });

  it("flashes a message and clears after the timeout", () => {
    const store = createFeedbackStore();
    store.flash("Copied", "success");
    expect(store.message?.text).toBe("Copied");
    expect(store.message?.kind).toBe("success");
    vi.advanceTimersByTime(3_500);
    expect(store.message).toBeNull();
  });

  it("replaces a previous message when a new one flashes", () => {
    const store = createFeedbackStore();
    store.flash("first", "info");
    store.flash("second", "error");
    expect(store.message?.text).toBe("second");
    expect(store.message?.kind).toBe("error");
  });

  it("clear() removes the message immediately", () => {
    const store = createFeedbackStore();
    store.flash("Copied", "success");
    store.clear();
    expect(store.message).toBeNull();
  });
});
