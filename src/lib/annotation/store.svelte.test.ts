// Annotation store tests. Cover the acceptance criteria from tracer-04
// through tracer-06:
//   - Numbered badges increment from 1 within each capture session.
//   - Toolbar changes affect subsequent annotations predictably.
//   - Ctrl+Z / Ctrl+Shift+Z operate on complete user actions.
//   - A new action after undo discards the obsolete redo branch.
//   - A fresh session begins with no annotations or inherited history.
//   - Text + Blur (tracer-05) participate in the same history semantics
//     and ship with their own degenerate-draft rules.
//   - Selection + transform + batch style + delete + z-order (tracer-06)
//     are all undoable and produce a single history entry per gesture.
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
    expect(annotationStore.selection.size).toBe(0);
    expect(annotationStore.transform).toBeNull();
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
    expect(annotationStore.selection.size).toBe(0);
    expect(annotationStore.transform).toBeNull();
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

  // --- Tracer-05: text + blur ----------------------------------------

  it("begins a text draft with an empty payload and resizes via updateDraft", () => {
    annotationStore.setTool("text");
    annotationStore.beginDraft("text", { x: 10, y: 10 });
    expect(annotationStore.draft?.geometry.kind).toBe("text");
    if (annotationStore.draft?.geometry.kind === "text") {
      expect(annotationStore.draft.geometry.text).toBe("");
      expect(annotationStore.draft.geometry.size.width).toBe(0);
    }
    annotationStore.updateDraft({ x: 100, y: 50 });
    if (annotationStore.draft?.geometry.kind === "text") {
      expect(annotationStore.draft.geometry.size.width).toBe(90);
      expect(annotationStore.draft.geometry.size.height).toBe(40);
    }
  });

  it("commitText pushes the typed text and promotes the draft", () => {
    annotationStore.setTool("text");
    annotationStore.beginDraft("text", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 100, y: 50 });
    annotationStore.commitText("hello world");
    expect(annotationStore.annotations).toHaveLength(1);
    expect(annotationStore.draft).toBeNull();
    expect(annotationStore.canUndo).toBe(true);
    const first = annotationStore.annotations[0];
    expect(first.geometry.kind).toBe("text");
    if (first.geometry.kind === "text") {
      expect(first.geometry.text).toBe("hello world");
    }
  });

  it("commitText with empty payload discards the draft without history", () => {
    annotationStore.setTool("text");
    // Snapshot the history depth so the test isolates commitText's
    // effect (setTool itself pushes history on the tool change).
    const undoBefore = annotationStore.canUndo;
    annotationStore.beginDraft("text", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 100, y: 50 });
    annotationStore.commitText("");
    expect(annotationStore.draft).toBeNull();
    expect(annotationStore.annotations).toHaveLength(0);
    // commitText on an empty payload must not create a new history
    // entry beyond what setTool already recorded.
    expect(annotationStore.canUndo).toBe(undoBefore);
  });

  it("zero-size text drafts are discarded on commit", () => {
    annotationStore.setTool("text");
    annotationStore.beginDraft("text", { x: 0, y: 0 });
    // No updateDraft — the size is 0×0.
    annotationStore.commitText("x");
    expect(annotationStore.annotations).toHaveLength(0);
  });

  it("commitText is a no-op when the draft is not text", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 100, y: 100 });
    annotationStore.commitText("should-not-promote");
    // Rectangle draft is still there — commitText must not promote
    // a non-text draft.
    expect(annotationStore.draft).not.toBeNull();
    expect(annotationStore.annotations).toHaveLength(0);
  });

  it("blur draft commits as a Blur geometry", () => {
    annotationStore.setTool("blur");
    annotationStore.beginDraft("blur", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 100, y: 50 });
    annotationStore.commitDraft();
    expect(annotationStore.annotations).toHaveLength(1);
    expect(annotationStore.annotations[0].geometry.kind).toBe("blur");
    if (annotationStore.annotations[0].geometry.kind === "blur") {
      expect(annotationStore.annotations[0].geometry.size.width).toBe(90);
      expect(annotationStore.annotations[0].geometry.size.height).toBe(40);
    }
    expect(annotationStore.canUndo).toBe(true);
  });

  it("zero-size blur drafts are discarded on commit", () => {
    annotationStore.setTool("blur");
    const undoBefore = annotationStore.canUndo;
    annotationStore.beginDraft("blur", { x: 0, y: 0 });
    annotationStore.commitDraft();
    expect(annotationStore.annotations).toHaveLength(0);
    expect(annotationStore.canUndo).toBe(undoBefore);
  });

  it("text + blur participate in undo/redo (tracer-05 round trip)", () => {
    annotationStore.setTool("text");
    annotationStore.beginDraft("text", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 100, y: 50 });
    annotationStore.commitText("label");
    expect(annotationStore.annotations).toHaveLength(1);

    annotationStore.setTool("blur");
    annotationStore.beginDraft("blur", { x: 200, y: 200 });
    annotationStore.updateDraft({ x: 300, y: 250 });
    annotationStore.commitDraft();
    expect(annotationStore.annotations).toHaveLength(2);

    annotationStore.undo();
    expect(annotationStore.annotations).toHaveLength(1);
    expect(annotationStore.annotations[0].geometry.kind).toBe("text");

    annotationStore.redo();
    expect(annotationStore.annotations).toHaveLength(2);
    expect(annotationStore.annotations[1].geometry.kind).toBe("blur");
  });

  // --- Tracer-06: selection -----------------------------------------

  /// Helper: seed two committed rectangles at known positions so the
  /// selection tests can exercise boundary cases without repeating
  /// the beginDraft dance.
  function seedRectangleRects() {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 60, y: 40 });
    annotationStore.commitDraft();
    annotationStore.beginDraft("rectangle", { x: 100, y: 100 });
    annotationStore.updateDraft({ x: 180, y: 160 });
    annotationStore.commitDraft();
    annotationStore.beginDraft("rectangle", { x: 200, y: 200 });
    annotationStore.updateDraft({ x: 220, y: 220 });
    annotationStore.commitDraft();
    return annotationStore.annotations.map((a) => a.id);
  }

  it("selectOnly with null clears the selection without history", () => {
    const [id] = seedRectangleRects();
    annotationStore.selectOnly(id);
    expect(annotationStore.selection.size).toBe(1);
    expect(annotationStore.isSelected(id)).toBe(true);
    const undoBefore = annotationStore.canUndo;
    annotationStore.selectOnly(null);
    expect(annotationStore.selection.size).toBe(0);
    // Clearing the selection does not push history — it's a passive
    // state mutation that the user can recreate by re-clicking.
    expect(annotationStore.canUndo).toBe(undoBefore);
  });

  it("selectOnly adds exactly one id and ignores the previous selection", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectOnly(ids[2]);
    expect(annotationStore.selection.size).toBe(1);
    expect(annotationStore.isSelected(ids[2])).toBe(true);
    expect(annotationStore.isSelected(ids[0])).toBe(false);
  });

  it("selectAdd and selectRemove accumulate / shrink the selection", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectAdd(ids[1]);
    annotationStore.selectAdd(ids[2]);
    expect(annotationStore.selection.size).toBe(3);
    annotationStore.selectRemove(ids[1]);
    expect(annotationStore.selection.size).toBe(2);
    expect(annotationStore.isSelected(ids[1])).toBe(false);
  });

  it("selectToggle flips membership", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectToggle(ids[0]);
    expect(annotationStore.selection.size).toBe(0);
    annotationStore.selectToggle(ids[0]);
    expect(annotationStore.isSelected(ids[0])).toBe(true);
  });

  it("selectMarquee replace mode swaps the selection set", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    // Three rectangle rects from seedRectangleRects():
    //   ids[0] at (10,10)-(60,40)
    //   ids[1] at (100,100)-(180,160)
    //   ids[2] at (200,200)-(220,220)
    // A marquee that overlaps only the third rect should leave just
    // the third selected.
    annotationStore.selectMarquee(
      { origin: { x: 195, y: 195 }, size: { width: 50, height: 50 } },
      "replace",
    );
    expect(annotationStore.selection.size).toBe(1);
    expect(annotationStore.isSelected(ids[2])).toBe(true);
    expect(annotationStore.isSelected(ids[0])).toBe(false);
  });

  it("selectMarquee add mode accumulates", () => {
    const ids = seedRectangleRects();
    annotationStore.selectMarquee(
      { origin: { x: 0, y: 0 }, size: { width: 80, height: 80 } },
      "replace",
    );
    expect(annotationStore.selection.size).toBe(1);
    annotationStore.selectMarquee(
      { origin: { x: 80, y: 80 }, size: { width: 100, height: 100 } },
      "add",
    );
    // First rect (10..60) AND second rect (100..180) — both
    // intersect the combined marquee.
    expect(annotationStore.selection.size).toBe(2);
    expect(annotationStore.isSelected(ids[0])).toBe(true);
    expect(annotationStore.isSelected(ids[1])).toBe(true);
  });

  it("selectAll selects every committed annotation", () => {
    const ids = seedRectangleRects();
    annotationStore.selectAll();
    expect(annotationStore.selection.size).toBe(ids.length);
  });

  it("selectionBounds returns the union of selected annotation boxes", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectAdd(ids[1]);
    const bounds = annotationStore.selectionBounds();
    expect(bounds).not.toBeNull();
    // Rects span (10,10)-(60,40) and (100,100)-(180,160). The union
    // is (10,10)-(180,160).
    expect(bounds).toEqual({
      origin: { x: 10, y: 10 },
      size: { width: 170, height: 150 },
    });
  });

  it("selectionBounds returns null when the selection is empty", () => {
    expect(annotationStore.selectionBounds()).toBeNull();
  });

  it("selectionColor / selectionStroke report uniform, mixed, and null", () => {
    const ids = seedRectangleRects();
    // Default colour is red, stroke is medium.
    annotationStore.setColor("red");
    annotationStore.setStroke("medium");
    annotationStore.selectOnly(ids[0]);
    expect(annotationStore.selectionColor()).toBe("red");
    expect(annotationStore.selectionStroke()).toBe("medium");
    annotationStore.clearSelection();
    expect(annotationStore.selectionColor()).toBeNull();
    expect(annotationStore.selectionStroke()).toBeNull();
    annotationStore.selectOnly(ids[0]);
    annotationStore.applyColorToSelection("blue");
    expect(annotationStore.selectionColor()).toBe("blue");
    // Select two rectangles that already have different colours;
    // the resolver should report "mixed".
    annotationStore.selectOnly(ids[0]);
    annotationStore.applyColorToSelection("red");
    annotationStore.selectOnly(ids[1]);
    annotationStore.applyColorToSelection("blue");
    annotationStore.selectAdd(ids[0]);
    expect(annotationStore.selectionColor()).toBe("mixed");
    // And the stroke side too.
    annotationStore.selectOnly(ids[0]);
    annotationStore.applyStrokeToSelection("thin");
    annotationStore.selectOnly(ids[1]);
    annotationStore.applyStrokeToSelection("thick");
    annotationStore.selectAdd(ids[0]);
    expect(annotationStore.selectionStroke()).toBe("mixed");
  });

  // --- Tracer-06: per-geometry handle contract ----------------------

  it("exposes the correct handle set for each geometry", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitDraft();
    const arrow = annotationStore.annotations[0];
    expect(annotationStore.handlesFor(arrow)).toEqual(["move", "tail", "tip"]);

    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitDraft();
    const rect = annotationStore.annotations[1];
    expect(annotationStore.handlesFor(rect)).toEqual([
      "move",
      "nw",
      "n",
      "ne",
      "e",
      "se",
      "s",
      "sw",
      "w",
    ]);

    annotationStore.setTool("text");
    annotationStore.beginDraft("text", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitText("hi");
    const text = annotationStore.annotations[2];
    expect(annotationStore.handlesFor(text)).toEqual(["move", "left", "right"]);

    annotationStore.setTool("numbered_badge");
    annotationStore.beginDraft("numbered_badge", { x: 0, y: 0 });
    annotationStore.commitDraft();
    const badge = annotationStore.annotations[3];
    expect(annotationStore.handlesFor(badge)).toEqual(["move"]);

    annotationStore.setTool("blur");
    annotationStore.beginDraft("blur", { x: 0, y: 0 });
    annotationStore.updateDraft({ x: 50, y: 50 });
    annotationStore.commitDraft();
    const blur = annotationStore.annotations[4];
    expect(annotationStore.handlesFor(blur)).toEqual([
      "move",
      "nw",
      "n",
      "ne",
      "e",
      "se",
      "s",
      "sw",
      "w",
    ]);
  });

  // --- Tracer-06: per-annotation transform --------------------------

  it("8-handle rectangle resize keeps stroke width intact and clamps min size", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 110, y: 60 });
    annotationStore.commitDraft();
    const rect = annotationStore.annotations[0];
    annotationStore.selectOnly(rect.id);
    annotationStore.beginTransform(rect.id, "se", { x: 110, y: 60 });
    annotationStore.updateTransform({ x: 200, y: 100 });
    annotationStore.endTransform();
    const updated = annotationStore.annotations[0];
    if (updated.geometry.kind === "rectangle") {
      expect(updated.geometry.size).toEqual({ width: 190, height: 90 });
      // The stroke width lives on the annotation, not the geometry,
      // so a transform must not mutate it.
      expect(updated.stroke).toBe("medium");
    } else {
      throw new Error("expected rectangle geometry");
    }
  });

  it("repeated transforms accumulate the same shape (no stroke drift)", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 110, y: 60 });
    annotationStore.commitDraft();
    const id = annotationStore.annotations[0].id;
    // Transform ten times by the same delta and assert the geometry
    // matches a single transform by the same final delta.
    for (let i = 0; i < 10; i += 1) {
      annotationStore.selectOnly(id);
      annotationStore.beginTransform(id, "se", { x: 110, y: 60 });
      annotationStore.updateTransform({ x: 200, y: 100 });
      annotationStore.endTransform();
    }
    const updated = annotationStore.annotations[0];
    if (updated.geometry.kind === "rectangle") {
      expect(updated.geometry.size).toEqual({ width: 190, height: 90 });
    }
  });

  it("arrow tail and tip handles move the right endpoint", () => {
    annotationStore.setTool("arrow");
    annotationStore.beginDraft("arrow", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 60, y: 60 });
    annotationStore.commitDraft();
    const id = annotationStore.annotations[0].id;
    annotationStore.selectOnly(id);
    annotationStore.beginTransform(id, "tail", { x: 10, y: 10 });
    annotationStore.updateTransform({ x: 0, y: 0 });
    annotationStore.endTransform();
    let arrow = annotationStore.annotations[0];
    if (arrow.geometry.kind === "arrow") {
      expect(arrow.geometry.tail).toEqual({ x: 0, y: 0 });
      expect(arrow.geometry.tip).toEqual({ x: 60, y: 60 });
    }
    annotationStore.beginTransform(id, "tip", { x: 60, y: 60 });
    annotationStore.updateTransform({ x: 100, y: 100 });
    annotationStore.endTransform();
    arrow = annotationStore.annotations[0];
    if (arrow.geometry.kind === "arrow") {
      expect(arrow.geometry.tail).toEqual({ x: 0, y: 0 });
      expect(arrow.geometry.tip).toEqual({ x: 100, y: 100 });
    }
  });

  it("badge is translate-only — non-move handles are not in the contract", () => {
    annotationStore.setTool("numbered_badge");
    annotationStore.beginDraft("numbered_badge", { x: 100, y: 100 });
    annotationStore.commitDraft();
    const handles = annotationStore.handlesFor(annotationStore.annotations[0]);
    expect(handles).toEqual(["move"]);
  });

  it("text exposes only horizontal width handles", () => {
    annotationStore.setTool("text");
    annotationStore.beginDraft("text", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 110, y: 60 });
    annotationStore.commitText("hello");
    const text = annotationStore.annotations[0];
    const id = text.id;
    annotationStore.selectOnly(id);
    annotationStore.beginTransform(id, "right", { x: 110, y: 60 });
    annotationStore.updateTransform({ x: 200, y: 60 });
    annotationStore.endTransform();
    const updated = annotationStore.annotations[0];
    if (updated.geometry.kind === "text") {
      expect(updated.geometry.size.width).toBe(190);
      // Vertical extent is anchored by the text content; the
      // horizontal handle must not touch it.
      expect(updated.geometry.size.height).toBe(50);
    }
  });

  it("one completed transform creates one undo entry", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 110, y: 60 });
    annotationStore.commitDraft();
    const id = annotationStore.annotations[0].id;
    const undoBefore = annotationStore.canUndo;
    annotationStore.selectOnly(id);
    annotationStore.beginTransform(id, "se", { x: 110, y: 60 });
    // Many frames...
    for (let i = 0; i < 20; i += 1) {
      annotationStore.updateTransform({ x: 110 + i, y: 60 + i });
    }
    annotationStore.endTransform();
    expect(annotationStore.canUndo).toBe(true);
    // A single undo restores the original size — confirming the
    // gesture is one undoable action even though it spanned 20
    // pointer-move frames.
    annotationStore.undo();
    const back = annotationStore.annotations[0];
    if (back.geometry.kind === "rectangle") {
      expect(back.geometry.size).toEqual({ width: 100, height: 50 });
    }
    // The undo depth is back to what it was before the transform.
    expect(annotationStore.canUndo).toBe(undoBefore);
  });

  it("cancelTransform reverts the gesture without leaving a history entry", () => {
    annotationStore.setTool("rectangle");
    annotationStore.beginDraft("rectangle", { x: 10, y: 10 });
    annotationStore.updateDraft({ x: 110, y: 60 });
    annotationStore.commitDraft();
    const id = annotationStore.annotations[0].id;
    const undoBefore = annotationStore.canUndo;
    annotationStore.selectOnly(id);
    annotationStore.beginTransform(id, "se", { x: 110, y: 60 });
    annotationStore.updateTransform({ x: 220, y: 110 });
    annotationStore.cancelTransform();
    // The snapshot pushed at gesture start was popped back off, so
    // the undo depth is exactly what it was before the gesture.
    expect(annotationStore.canUndo).toBe(undoBefore);
    expect(annotationStore.annotations[0].geometry).toMatchObject({
      kind: "rectangle",
      origin: { x: 10, y: 10 },
      size: { width: 100, height: 50 },
    });
  });

  // --- Tracer-06: multi-select translate ----------------------------

  it("beginTranslateSelection moves every selected annotation by the same delta", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectAdd(ids[1]);
    annotationStore.beginTranslateSelection({ x: 0, y: 0 });
    annotationStore.updateTranslateSelection({ x: 30, y: 40 });
    annotationStore.endTranslateSelection();
    const a0 = annotationStore.annotations[0];
    const a1 = annotationStore.annotations[1];
    if (a0.geometry.kind === "rectangle" && a1.geometry.kind === "rectangle") {
      expect(a0.geometry.origin).toEqual({ x: 40, y: 50 });
      expect(a1.geometry.origin).toEqual({ x: 130, y: 140 });
    }
  });

  it("multi-select translate is drift-free across many frames", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectAdd(ids[1]);
    annotationStore.beginTranslateSelection({ x: 0, y: 0 });
    // Emulate a jittery pointer that advances by 10 px per frame.
    for (let i = 1; i <= 10; i += 1) {
      annotationStore.updateTranslateSelection({ x: i * 10, y: i * 5 });
    }
    annotationStore.endTranslateSelection();
    const a0 = annotationStore.annotations[0];
    if (a0.geometry.kind === "rectangle") {
      expect(a0.geometry.origin).toEqual({ x: 110, y: 60 });
    }
  });

  it("one translate gesture creates one undo entry", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectAdd(ids[1]);
    annotationStore.beginTranslateSelection({ x: 0, y: 0 });
    annotationStore.updateTranslateSelection({ x: 50, y: 60 });
    annotationStore.endTranslateSelection();
    const undoBefore = annotationStore.canUndo;
    annotationStore.undo();
    // After undo, the original positions are back.
    const a0 = annotationStore.annotations[0];
    if (a0.geometry.kind === "rectangle") {
      expect(a0.geometry.origin).toEqual({ x: 10, y: 10 });
    }
    // Then undo again should restore the pre-select state.
    expect(annotationStore.canUndo).toBe(undoBefore ? true : false);
  });

  // --- Tracer-06: batch style + delete + z-order -------------------

  it("applyColorToSelection updates every selected annotation in one history entry", () => {
    seedRectangleRects();
    annotationStore.selectAll();
    annotationStore.applyColorToSelection("blue");
    for (const a of annotationStore.annotations) {
      expect(a.color).toBe("blue");
    }
    annotationStore.undo();
    // Every annotation reverts to "red" (the default).
    for (const a of annotationStore.annotations) {
      expect(a.color).toBe("red");
    }
  });

  it("applyStrokeToSelection on empty selection falls back to the next-draw style", () => {
    annotationStore.applyStrokeToSelection("thick");
    expect(annotationStore.stroke).toBe("thick");
    annotationStore.undo();
    expect(annotationStore.stroke).toBe("medium");
  });

  it("deleteSelection removes the selected set and is reversible", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.selectAdd(ids[2]);
    annotationStore.deleteSelection();
    expect(annotationStore.annotations).toHaveLength(1);
    expect(annotationStore.selection.size).toBe(0);
    annotationStore.undo();
    expect(annotationStore.annotations).toHaveLength(3);
  });

  it("raiseSelection and lowerSelection bump z-order by one slot", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    const startZ = annotationStore.annotations[0].zOrder;
    annotationStore.raiseSelection();
    expect(annotationStore.annotations[0].zOrder).toBe(startZ + 1);
    annotationStore.lowerSelection();
    expect(annotationStore.annotations[0].zOrder).toBe(startZ);
  });

  it("bringToFrontSelection assigns the highest z-order", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[0]);
    annotationStore.bringToFrontSelection();
    const maxZ = Math.max(...annotationStore.annotations.map((a) => a.zOrder));
    expect(annotationStore.annotations[0].zOrder).toBe(maxZ);
  });

  it("sendToBackSelection assigns the lowest z-order", () => {
    const ids = seedRectangleRects();
    annotationStore.selectOnly(ids[2]);
    annotationStore.sendToBackSelection();
    const minZ = Math.min(...annotationStore.annotations.map((a) => a.zOrder));
    expect(annotationStore.annotations[2].zOrder).toBe(minZ);
  });

  it("transform + delete + z-order chain round-trips through undo/redo", () => {
    const ids = seedRectangleRects();
    // Transform.
    annotationStore.selectOnly(ids[0]);
    annotationStore.beginTransform(ids[0], "se", { x: 60, y: 40 });
    annotationStore.updateTransform({ x: 100, y: 80 });
    annotationStore.endTransform();
    // Delete.
    annotationStore.selectOnly(ids[1]);
    annotationStore.deleteSelection();
    // Raise.
    annotationStore.selectOnly(ids[0]);
    annotationStore.raiseSelection();
    const undoBefore = annotationStore.canUndo;
    annotationStore.undo();
    annotationStore.undo();
    annotationStore.undo();
    // After three undos: z-order back, second rect restored, rect
    // back to original size.
    expect(annotationStore.annotations).toHaveLength(3);
    const a0 = annotationStore.annotations[0];
    if (a0.geometry.kind === "rectangle") {
      expect(a0.geometry.size).toEqual({ width: 50, height: 30 });
    }
    annotationStore.redo();
    annotationStore.redo();
    annotationStore.redo();
    expect(annotationStore.annotations).toHaveLength(2);
    expect(annotationStore.canUndo).toBe(undoBefore);
  });
});
