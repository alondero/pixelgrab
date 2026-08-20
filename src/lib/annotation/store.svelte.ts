// Annotation store. Owns the editor's view of the capture: active tool,
// active style, the annotation list, the badge counter, and the
// semantic undo/redo history. The Rust core receives the finalized
// annotation list at commit time and flattens it onto the frozen
// framebuffer (see `crates/pixelgrab-contracts/src/annotation.rs`).
//
// History semantics:
//   - Every "completed" user action — drawing pointerup, style change,
//     tool switch, delete — pushes the *pre-mutation* snapshot onto
//     `past`.
//   - `undo()` pops `past`, pushes the current state onto `future`,
//     and restores the popped snapshot. `redo()` is the mirror.
//   - Any new mutation while `future` is non-empty clears `future`.
//     This matches the spec's "A new action after undo discards the
//     obsolete redo branch."
//   - Pointer-move frames do not enter history because they only
//     mutate the in-flight `draft`, never the committed `annotations`
//     array. The history is only mutated when a finalized shape is
//     promoted or when the toolbar style changes.

import type {
  Annotation,
  AnnotationColor,
  AnnotationGeometry,
  AnnotationStroke,
  AnnotationTool,
} from "$lib/ipc/types";

/// Default badge radius in physical pixels. Mirrors the
/// `BADGE_RADIUS_PX` constant in the contracts crate; both must agree
/// so the editor's preview and the rasterizer's painted circle stay
/// coincident.
export const BADGE_RADIUS_PX = 18;

/// Snapshot of the user-controlled editor state. The history captures
/// exactly this shape so undo / redo restore every aspect of the
/// editor that a user can change: the annotation list, the active
/// tool, the active colour, and the active stroke. The badge counter
/// is session-scoped — it survives undo / redo so a redrawn badge
/// can keep its place in the sequence.
interface Snapshot {
  annotations: Annotation[];
  tool: AnnotationTool;
  color: AnnotationColor;
  stroke: AnnotationStroke;
}

interface AnnotationStore {
  tool: AnnotationTool;
  color: AnnotationColor;
  stroke: AnnotationStroke;
  annotations: Annotation[];
  draft: Annotation | null;
  nextId: number;
  badgeCounter: number;
  history: {
    past: Snapshot[];
    future: Snapshot[];
  };
}

function defaultState(): AnnotationStore {
  return {
    tool: "select",
    color: "red",
    stroke: "medium",
    annotations: [],
    draft: null,
    nextId: 1,
    badgeCounter: 1,
    history: { past: [], future: [] },
  };
}

function snapshotOf(state: AnnotationStore): Snapshot {
  // `$state.snapshot` returns plain values from the proxy so the
  // history stack stays free of reactivity.
  return {
    annotations: $state.snapshot(state.annotations) as Annotation[],
    tool: state.tool,
    color: state.color,
    stroke: state.stroke,
  };
}

function createAnnotationStore() {
  const inner: AnnotationStore = $state(defaultState());

  function pushHistory() {
    // Capture the *pre*-mutation snapshot. Pointer-move events never
    // reach this function because they only mutate `inner.draft`, not
    // `inner.annotations` or the toolbar style.
    inner.history.past.push(snapshotOf(inner));
    // New action invalidates the redo branch.
    inner.history.future = [];
    // Bound the history depth so a runaway session cannot grow the
    // stack unboundedly. 256 is generous — a real session rarely
    // exceeds a few dozen actions.
    if (inner.history.past.length > 256) {
      inner.history.past.splice(0, inner.history.past.length - 256);
    }
  }

  function nextId(): number {
    const id = inner.nextId;
    inner.nextId += 1;
    return id;
  }

  /// Promote the in-flight draft to the committed annotation list.
  /// Internal helper that performs the actual array push; the public
  /// `commitDraft` method handles degenerate-draft validation.
  function promoteDraft() {
    if (!inner.draft) return;
    if (!inner.annotations.includes(inner.draft)) {
      inner.annotations.push(inner.draft);
    }
    inner.draft = null;
  }

  /// Validate that the draft is not degenerate (a zero-length arrow /
  /// rectangle has no visible shape and would clutter the export).
  /// Returns true when the draft should be discarded.
  function isDraftDegenerate(): boolean {
    const draft = inner.draft;
    if (!draft) return true;
    if (draft.geometry.kind === "arrow") {
      const dx = draft.geometry.tip.x - draft.geometry.tail.x;
      const dy = draft.geometry.tip.y - draft.geometry.tail.y;
      return Math.hypot(dx, dy) < 4;
    }
    if (draft.geometry.kind === "rectangle") {
      return draft.geometry.size.width < 4 || draft.geometry.size.height < 4;
    }
    return false;
  }

  return {
    // Read-only getters so consumers cannot accidentally mutate the
    // proxy fields outside the store methods.
    get tool() {
      return inner.tool;
    },
    get color() {
      return inner.color;
    },
    get stroke() {
      return inner.stroke;
    },
    get annotations() {
      return inner.annotations;
    },
    get draft() {
      return inner.draft;
    },
    get badgeCounter() {
      return inner.badgeCounter;
    },
    get canUndo() {
      return inner.history.past.length > 0;
    },
    get canRedo() {
      return inner.history.future.length > 0;
    },

    /// Switch tools. Style changes count as completed actions in the
    /// history so a tool change followed by an undo restores the
    /// previous tool (per the spec's "semantic undo ... for completed
    /// drawing and style actions").
    setTool(tool: AnnotationTool) {
      if (inner.tool === tool) return;
      pushHistory();
      inner.tool = tool;
    },
    setColor(color: AnnotationColor) {
      if (inner.color === color) return;
      pushHistory();
      inner.color = color;
    },
    setStroke(stroke: AnnotationStroke) {
      if (inner.stroke === stroke) return;
      pushHistory();
      inner.stroke = stroke;
    },

    /// Begin a draft annotation for the active tool. Called on
    /// pointerdown of an arrow, rectangle, or badge. The geometry is
    /// set to a zero-shape initial value; the pointermove handler
    /// updates it incrementally.
    beginDraft(kind: AnnotationGeometry["kind"], point: { x: number; y: number }) {
      const id = nextId();
      const geometry: AnnotationGeometry =
        kind === "arrow"
          ? { kind: "arrow", tail: point, tip: point }
          : kind === "rectangle"
            ? { kind: "rectangle", origin: point, size: { width: 0, height: 0 } }
            : { kind: "numbered_badge", center: point, radius: BADGE_RADIUS_PX };
      const annotation: Annotation = {
        id,
        geometry,
        color: inner.color,
        stroke: inner.stroke,
        zOrder: inner.annotations.length + 1,
        ...(kind === "numbered_badge" ? { number: inner.badgeCounter } : {}),
      };
      inner.draft = annotation;
      if (kind === "numbered_badge") {
        inner.badgeCounter += 1;
      }
    },

    /// Update the in-flight draft. No history entry — pointer-move
    /// frames do not enter history.
    updateDraft(point: { x: number; y: number }) {
      const draft = inner.draft;
      if (!draft) return;
      if (draft.geometry.kind === "arrow") {
        draft.geometry.tip = point;
      } else if (draft.geometry.kind === "rectangle") {
        draft.geometry.size = {
          width: Math.abs(point.x - draft.geometry.origin.x),
          height: Math.abs(point.y - draft.geometry.origin.y),
        };
        if (point.x < draft.geometry.origin.x) {
          draft.geometry.origin = { x: point.x, y: draft.geometry.origin.y };
        }
        if (point.y < draft.geometry.origin.y) {
          draft.geometry.origin = { x: draft.geometry.origin.x, y: point.y };
        }
      } else {
        draft.geometry.center = point;
      }
    },

    /// Finalize the draft annotation. Pushes a history entry covering
    /// the *previous* annotation list, then promotes the draft.
    /// Zero-area arrows and rectangles are discarded instead of
    /// committed so a stray click does not leave a phantom annotation.
    commitDraft(): void {
      const draft = inner.draft;
      if (!draft) return;
      if (isDraftDegenerate()) {
        inner.draft = null;
        return;
      }
      pushHistory();
      promoteDraft();
    },

    /// Discard the current draft (pointerup with a zero-length shape,
    /// or Escape pressed mid-draw).
    cancelDraft() {
      inner.draft = null;
    },

    /// Undo the most recent committed action. Returns true on success.
    /// Restores the annotation list, the active tool, the colour, and
    /// the stroke from the popped snapshot. The badge counter is
    /// session-scoped and is intentionally NOT restored.
    undo(): boolean {
      const past = inner.history.past;
      if (past.length === 0) return false;
      const previous = past.pop()!;
      inner.history.future.push(snapshotOf(inner));
      inner.annotations = previous.annotations;
      inner.tool = previous.tool;
      inner.color = previous.color;
      inner.stroke = previous.stroke;
      // Drop any in-flight draft so the editor is in a consistent
      // state after the undo.
      inner.draft = null;
      return true;
    },

    /// Redo the most recently undone action. Returns true on success.
    redo(): boolean {
      const future = inner.history.future;
      if (future.length === 0) return false;
      const next = future.pop()!;
      inner.history.past.push(snapshotOf(inner));
      inner.annotations = next.annotations;
      inner.tool = next.tool;
      inner.color = next.color;
      inner.stroke = next.stroke;
      inner.draft = null;
      return true;
    },

    /// Reset every store field. Called on session cleanup so a fresh
    /// session starts with no annotations, no badge counter carry,
    /// and no history.
    reset(): void {
      Object.assign(inner, defaultState());
    },
  };
}

/// Single store instance shared by every overlay view. The store is
/// created lazily so test files can `reset()` it between scenarios
/// without touching global state.
export const annotationStore = createAnnotationStore();
