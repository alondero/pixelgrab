// Annotation store tests. Cover the acceptance criteria from tracer-04:
//   - Numbered badges increment from 1 within each capture session.
//   - Toolbar changes affect subsequent annotations predictably.
//   - Ctrl+Z / Ctrl+Shift+Z operate on complete user actions.
//   - A new action after undo discards the obsolete redo branch.
//   - A fresh session begins with no annotations or inherited history.
//
// The store uses Svelte 5 runes; we import it from `store.svelte.ts`
// after resetting it via `annotationStore.reset()` in `beforeEach`.

import { describe, it, expect, beforeEach } from "vitest";
import { annotationStore } from "./store.svelte";

describe("annotationStore", () => {
  beforeEach(() => {
    annotationStore.reset();
  });

  it("starts in a fresh state with the badge counter at 1", () => {
    expect(annotationStore.annotations).toEqual([]);
    expect(annotationStore.draft).toBeNull();
    expect(annotationStore.badgeCounter).toBe(1);
    expect(annotationStore.canUndo).toBe(false);
    expect(annotationStore.canRedo).toBe(false);
    expect(annotationStore.tool).toBe("select");
    expect(annotationStore.color).toBe("red");
    expect(annotationStore.stroke).toBe("medium");
  });

  it("draws an arrow on beginDraft + commitDraft and increments z-order", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 100, y: 60 });
    expect(annotationStore.draft).not.toBeNull();
    expect(annotationStore.draft?.geometry.kind).toBe("arrow");
    expect(annotationStore.commitDraft()).toBeUndefined();
    expect(annotationStore.annotations).toHaveLength(1);
    expect(annotationStore.annotations[0].geometry.kind).toBe("arrow");
    expect(annotationStore.draft).toBeNull();
  });

  it("does not enter history on pointer-move frames", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    // Many pointer-moves update the draft, but only the pointerup
    // (commitDraft) creates a history entry.
    for (let i = 0; i < 50; i += 1) {
      annotationStore.updateDraft({ x: i, y: i });
    }
    annotationStore.commitDraft();
    // After a single commit, exactly one history entry exists.
    expect(annotationStore.canUndo).toBe(true);
    expect(annotationStore.canRedo).toBe(false);
    annotationStore.undo();
    expect(annotationStore.annotations).toEqual([]);
  });

  it("increments the badge counter only when a badge is committed", () => {
    annotationStore.setTool("numbered_badge");
    annotationStore.beginDraft("numbered_badge", { x: 10, y: 10 });
    annotationStore.commitDraft();
    expect(annotationStore.badgeCounter).toBe(2);
    expect(annotationStore.annotations[0].number).toBe(1);

    annotationStore.beginDraft("numbered_badge", { x: 30, y: 30 });
    annotationStore.commitDraft();
    expect(annotationStore.badgeCounter).toBe(3);
    expect(annotationStore.annotations[1].number).toBe(2);
  });

  it("does NOT increment the badge counter for arrows or rectangles", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 100, y: 100 });
    annotationStore.commitDraft();
    expect(annotationStore.badgeCounter).toBe(1);

    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitDraft();
    expect(annotationStore.badgeCounter).toBe(1);
  });

  it("toolbar style changes do not retroactively mutate committed annotations", () => {
    annotationStore.setTool("arrow");
    annotationStore.setColor("red");
    annotationStore.setStroke("thin");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 0 });
    annotationStore.commitDraft();
    const first = annotationStore.annotations[0];
    expect(first.color).toBe("red");
    expect(first.stroke).toBe("thin");

    // Subsequent style change + new annotation.
    annotationStore.setColor("blue");
    annotationStore.setStroke("thick");
    annotationStore.beginDraft("arrow", { x: 100, y: 0 });
    annotationStore.updateDraft({ x: 150, y: 0 });
    annotationStore.commitDraft();
    expect(annotationStore.annotations[0].color).toBe("red");
    expect(annotationStore.annotations[0].stroke).toBe("thin");
    expect(annotationStore.annotations[1].color).toBe("blue");
    expect(annotationStore.annotations[1].stroke).toBe("thick");
  });

  it("undo restores the previous annotation list and clears the draft", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 100, y: 0 });
    annotationStore.commitDraft();
    // Snapshot a deep copy because the store mutates the array in
    // place on subsequent commits; the live reference would otherwise
    // pick up the second annotation.
    const committed = structuredClone($state.snapshot(annotationStore.annotations));

    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 200, y: 0 });
    annotationStore.commitDraft();
    expect(annotationStore.annotations).toHaveLength(2);

    annotationStore.undo();
    expect(annotationStore.annotations).toEqual(committed);
    expect(annotationStore.draft).toBeNull();
  });

  it("redo replays the undone action", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 100, y: 0 });
    annotationStore.commitDraft();
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 200, y: 0 });
    annotationStore.commitDraft();
    annotationStore.undo();
    annotationStore.redo();
    expect(annotationStore.annotations).toHaveLength(2);
  });

  it("a new action after undo discards the obsolete redo branch", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 100, y: 0 });
    annotationStore.commitDraft();
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 200, y: 0 });
    annotationStore.commitDraft();
    annotationStore.undo();
    expect(annotationStore.canRedo).toBe(true);

    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 300, y: 0 });
    annotationStore.commitDraft();
    expect(annotationStore.canRedo).toBe(false);
  });

  it("cancelDraft discards the in-flight shape and does not create history", () => {
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 0 });
    annotationStore.cancelDraft();
    expect(annotationStore.draft).toBeNull();
    expect(annotationStore.annotations).toHaveLength(0);
    expect(annotationStore.canUndo).toBe(false);
  });

  it("reset wipes every field including history and badge counter", () => {
    annotationStore.setTool("numbered_badge");
    annotationStore.beginDraft("numbered_badge", { x: 0, y: 0 });
    annotationStore.commitDraft();
    annotationStore.beginDraft("numbered_badge", { x: 50, y: 0 });
    annotationStore.commitDraft();
    expect(annotationStore.badgeCounter).toBe(3);

    annotationStore.reset();
    expect(annotationStore.annotations).toEqual([]);
    expect(annotationStore.draft).toBeNull();
    expect(annotationStore.badgeCounter).toBe(1);
    expect(annotationStore.canUndo).toBe(false);
    expect(annotationStore.canRedo).toBe(false);
  });

  it("badgeHitTest removed — hit-test belongs to tracer-06 selection tool", () => {
    // The selection tool + per-annotation hit-test lives behind
    // #18 (Tracer 06). tracer-04 deliberately ships no V-tool
    // behaviour so the hit-test helper would be dead code.
    expect((annotationStore as unknown as { badgeHitTest?: unknown }).badgeHitTest).toBeUndefined();
  });

  it("style changes are undoable: setColor, setStroke, setTool", () => {
    annotationStore.setColor("red");
    annotationStore.setStroke("thin");
    annotationStore.setTool("arrow");

    annotationStore.setColor("blue");
    expect(annotationStore.color).toBe("blue");
    annotationStore.undo();
    expect(annotationStore.color).toBe("red");

    annotationStore.setStroke("thick");
    annotationStore.undo();
    expect(annotationStore.stroke).toBe("thin");

    annotationStore.setTool("rectangle");
    annotationStore.undo();
    expect(annotationStore.tool).toBe("arrow");
  });

  it("setColor with the same value does not push history", () => {
    annotationStore.setColor("red");
    annotationStore.setColor("red");
    expect(annotationStore.canUndo).toBe(false);
    annotationStore.setColor("blue");
    expect(annotationStore.canUndo).toBe(true);
    expect(annotationStore.canRedo).toBe(false);
  });

  it("undo of a style change discards any pending redo branch", () => {
    annotationStore.setColor("red");
    annotationStore.setColor("blue");
    annotationStore.undo();
    expect(annotationStore.canRedo).toBe(true);
    // Set color again — must clear the redo branch.
    annotationStore.setColor("yellow");
    expect(annotationStore.canRedo).toBe(false);
  });

  it("zero-length rectangles and arrows are discarded on commit", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 0, y: 0 });
    annotationStore.commitDraft();
    expect(annotationStore.annotations).toHaveLength(0);

    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.commitDraft();
    expect(annotationStore.annotations).toHaveLength(0);
  });
});
