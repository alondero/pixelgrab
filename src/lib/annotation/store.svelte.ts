// Annotation store. Owns the editor's view of the capture: active tool,
// active style, the annotation list, the badge counter, the selection
// set, and the semantic undo/redo history. The Rust core receives the
// finalized annotation list at commit time and flattens it onto the
// frozen framebuffer (see `crates/pixelgrab-contracts/src/annotation.rs`).
//
// History semantics:
//   - Every "completed" user action — drawing pointerup, style change,
//     tool switch, delete, selection, transform, z-order — pushes the
//     *pre-mutation* snapshot onto `past`.
//   - `undo()` pops `past`, pushes the current state onto `future`,
//     and restores the popped snapshot. `redo()` is the mirror.
//   - Any new mutation while `future` is non-empty clears `future`.
//     This matches the spec's "A new action after undo discards the
//     obsolete redo branch."
//   - Pointer-move frames do not enter history because they only
//     mutate the in-flight `draft`, never the committed `annotations`
//     array. The history is only mutated when a finalized shape is
//     promoted or when the toolbar style changes.
//   - Transform gestures follow the same pattern: `beginTransform`
//     snapshots once, every `updateTransform` mutates the live entity
//     without history, and `endTransform` pushes the single
//     pre-mutation snapshot. Multi-select translate is structurally
//     identical (`beginTranslateSelection` / `updateTranslateSelection`
//     / `endTranslateSelection`).

import type {
  Annotation,
  AnnotationColor,
  AnnotationGeometry,
  AnnotationStroke,
  AnnotationTool,
  PhysicalPoint,
  PhysicalSize,
} from "$lib/ipc/types";

/// Default badge radius in physical pixels. Mirrors the
/// `BADGE_RADIUS_PX` constant in the contracts crate; both must agree
/// so the editor's preview and the rasterizer's painted circle stay
/// coincident.
export const BADGE_RADIUS_PX = 18;

/// Default blur radius (half-extent of the box-blur kernel). The
/// Rust rasterizer uses `radius = 4` (9×9 kernel) for a strong but
/// cheap redaction. Mirrored here so the editor's preview and the
/// flattened PNG agree.
export const DEFAULT_BLUR_RADIUS = 4;

/// Minimum handle delta (physical pixels) used during 8-handle
/// rectangle / blur resize so a transform cannot collapse the box
/// below the renderer-visible threshold.
const MIN_BOX_DIMENSION = 4;

/// Transform handle identifiers. Each geometry exposes only its
/// resolved handle semantics:
///
///   - Arrow → `tail` + `tip` (the two endpoints). The 8-handle
///     contract does not apply because the arrow has no axis-aligned
///     bounding box to maintain during a resize.
///   - Rectangle / Blur → the conventional 8 handles (`nw..w`).
///   - Text → `left` + `right` only. The vertical extent is governed
///     by the text content at render time, so a vertical handle would
///     silently produce a misaligned plate after the next wrap.
///   - Numbered badge → translate-only (`move`). The radius is a
///     fixed session constant, so resize handles would be misleading.
///   - `move` is also the universal translate handle used by every
///     geometry for body-drag translation.
export type TransformHandle =
  | "nw"
  | "n"
  | "ne"
  | "e"
  | "se"
  | "s"
  | "sw"
  | "w"
  | "tail"
  | "tip"
  | "left"
  | "right"
  | "move";

/// Axis-aligned bounding box in physical pixels. The store
/// computes the union of selected annotations so the overlay can
/// render a single selection rectangle instead of one per item.
export interface PhysicalRect {
  /** Top-left corner. */
  origin: PhysicalPoint;
  /** Width / height in physical pixels. */
  size: PhysicalSize;
}

/// The resolved colour + stroke state for the current selection.
/// `null` for either field means "no selection" (the toolbar falls
/// back to the next-draw style). The literal `"mixed"` value means
/// the selected set spans more than one value; the toolbar renders
/// a mixed-state indicator and a batch edit applies to every
/// compatible annotation in a single history entry.
export type SelectionColorState = AnnotationColor | "mixed" | null;
export type SelectionStrokeState = AnnotationStroke | "mixed" | null;

/// Snapshot of the user-controlled editor state. The history captures
/// exactly this shape so undo / redo restore every aspect of the
/// editor that a user can change: the annotation list, the active
/// tool, the active colour, the active stroke, and the selection set.
/// The badge counter is session-scoped — it survives undo / redo so
/// a redrawn badge can keep its place in the sequence.
interface Snapshot {
  annotations: Annotation[];
  tool: AnnotationTool;
  color: AnnotationColor;
  stroke: AnnotationStroke;
  selectedIds: number[];
}

/// In-flight transform state. Populated by `beginTransform` /
/// `beginTranslateSelection`; mutated by every `updateTransform` /
/// `updateTranslateSelection` frame; cleared by `endTransform` /
/// `cancelTransform`. The snapshot is the pre-mutation state so the
/// gesture is a single undoable action even though it spans many
/// pointer-move frames.
interface TransformState {
  kind: "transform" | "translate";
  /// The pre-mutation snapshot to push onto `past` when the gesture
  /// completes. Stored by value (via `$state.snapshot`) so the
  /// subsequent mutations cannot mutate the history entry.
  snapshot: Snapshot;
  /// The set of annotation ids the gesture is mutating. The id list
  /// is captured at gesture start so an undo + redo of a partial
  /// gesture does not reach across ids that joined later.
  ids: number[];
  /// For single-annotation transforms: the handle the user grabbed.
  /// For translate: `undefined` (the body-drag is the only meaning).
  handle?: TransformHandle;
  /// The cursor position at gesture start, in physical pixels. Used
  /// to compute the gesture delta so every frame is projected
  /// against the original geometry (no frame-to-frame drift).
  startCursor: PhysicalPoint;
  /// The single-annotation's geometry at gesture start, deep-copied.
  /// Populated for `kind === "transform"`; `null` for translate
  /// gestures (which carry per-id geometry in `initialGeometryMap`).
  initialGeometry: AnnotationGeometry | null;
  /// Per-annotation geometry at gesture start, deep-copied. Populated
  /// for `kind === "translate"` so a multi-select move applies the
  /// same delta-from-origin to every selected annotation without
  /// frame-to-frame drift.
  initialGeometryMap: Map<number, AnnotationGeometry> | null;
}

interface AnnotationStore {
  tool: AnnotationTool;
  color: AnnotationColor;
  stroke: AnnotationStroke;
  annotations: Annotation[];
  draft: Annotation | null;
  nextId: number;
  badgeCounter: number;
  selection: Set<number>;
  history: {
    past: Snapshot[];
    future: Snapshot[];
  };
  /// Active transform gesture, or `null` when no drag is in flight.
  /// Stored as a single field so the store stays a flat $state
  /// proxy (runes do not nest typed objects cleanly).
  transform: TransformState | null;
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
    selection: new Set<number>(),
    history: { past: [], future: [] },
    transform: null,
  };
}

function snapshotOf(state: AnnotationStore): Snapshot {
  // `$state.snapshot` returns plain values from the proxy so the
  // history stack stays free of reactivity. The selection set is
  // coerced to an array so the snapshot is JSON-serializable and
  // Svelte's deep-clone semantics give every entry its own copy.
  return {
    annotations: $state.snapshot(state.annotations) as Annotation[],
    tool: state.tool,
    color: state.color,
    stroke: state.stroke,
    selectedIds: Array.from(state.selection),
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
    if (draft.geometry.kind === "text") {
      // Text without content is degenerate; the box must also have
      // area so the plate is visible. The overlay commits via
      // `commitText` which sets the text payload; if the user cancels
      // without typing, `cancelDraft` discards the in-flight box.
      if (!draft.geometry.text) return true;
      return draft.geometry.size.width < 4 || draft.geometry.size.height < 4;
    }
    if (draft.geometry.kind === "blur") {
      return draft.geometry.size.width < 4 || draft.geometry.size.height < 4;
    }
    return false;
  }

  /// Find an annotation by id. Returns the live (proxy) reference so
  /// mutations through the helper fields propagate to the array.
  function findById(id: number): Annotation | undefined {
    return inner.annotations.find((a) => a.id === id);
  }

  /// Compute the axis-aligned bounding box of an annotation in physical
  /// pixels. The result is the smallest box that contains every drawn
  /// pixel of the entity (or, for blur, the rect itself). Used by the
  /// overlay to render a single selection rectangle and to compute the
  /// marquee-intersection test.
  function annotationBounds(annotation: Annotation): PhysicalRect {
    const g = annotation.geometry;
    if (g.kind === "arrow") {
      const x = Math.min(g.tail.x, g.tip.x);
      const y = Math.min(g.tail.y, g.tip.y);
      const right = Math.max(g.tail.x, g.tip.x);
      const bottom = Math.max(g.tail.y, g.tip.y);
      return {
        origin: { x, y },
        size: { width: Math.max(0, right - x), height: Math.max(0, bottom - y) },
      };
    }
    if (g.kind === "rectangle" || g.kind === "text" || g.kind === "blur") {
      return {
        origin: { x: g.origin.x, y: g.origin.y },
        size: { width: g.size.width, height: g.size.height },
      };
    }
    // Numbered badge: a circle around `center` with `radius`.
    return {
      origin: { x: g.center.x - g.radius, y: g.center.y - g.radius },
      size: { width: g.radius * 2, height: g.radius * 2 },
    };
  }

  /// Format-agnostic axis-aligned bounding box intersection. Two
  /// rects `a` and `b` intersect iff they overlap on both axes.
  function rectIntersects(a: PhysicalRect, b: PhysicalRect): boolean {
    return !(
      a.origin.x + a.size.width < b.origin.x ||
      b.origin.x + b.size.width < a.origin.x ||
      a.origin.y + a.size.height < b.origin.y ||
      b.origin.y + b.size.height < a.origin.y
    );
  }

  /// Return the list of selected annotations (live references).
  function selectedAnnotations(): Annotation[] {
    const ids = inner.selection;
    const out: Annotation[] = [];
    for (const ann of inner.annotations) {
      if (ids.has(ann.id)) out.push(ann);
    }
    return out;
  }

  /// Coerce a physical-point value into a concrete point. The
  /// transform helpers consume physical-pixel coordinates so a
  /// trivially-clamped pointer stays inside the crop without crashing
  /// a wandering cursor.
  function clampPoint(p: PhysicalPoint): PhysicalPoint {
    return { x: Math.round(p.x), y: Math.round(p.y) };
  }

  /// Restore the editor's mutable state from a snapshot. Used by
  /// `undo`, `redo`, and `cancelTransform`. The selection set is
  /// replaced wholesale so the proxy triggers reactivity.
  function restoreSnapshot(snap: Snapshot) {
    inner.annotations = snap.annotations;
    inner.tool = snap.tool;
    inner.color = snap.color;
    inner.stroke = snap.stroke;
    inner.selection.clear();
    for (const id of snap.selectedIds) {
      inner.selection.add(id);
    }
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
    get selection() {
      return inner.selection;
    },
    get canUndo() {
      return inner.history.past.length > 0;
    },
    get canRedo() {
      return inner.history.future.length > 0;
    },
    /// Live reference to the in-flight transform gesture. `null`
    /// when the editor is idle; populated by `beginTransform` /
    /// `beginTranslateSelection`; cleared by `endTransform` /
    /// `cancelTransform`. The overlay uses this to switch the
    /// cursor and to skip marquee selection while a drag is in
    /// flight.
    get transform() {
      return inner.transform;
    },

    /// Switch tools. Style changes count as completed actions in the
    /// history so a tool change followed by an undo restores the
    /// previous tool (per the spec's "semantic undo ... for completed
    /// drawing and style actions").
    setTool(tool: AnnotationTool) {
      if (inner.tool === tool) return;
      pushHistory();
      inner.tool = tool;
      // Switching to a drawing tool implies attention to a fresh
      // shape; clear the selection so the user does not accidentally
      // apply a batch style to an existing annotation.
      if (tool !== "select") {
        inner.selection.clear();
      }
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
    /// pointerdown of an arrow, rectangle, badge, text, or blur.
    /// The geometry is set to a zero-shape initial value; the
    /// pointermove handler updates it incrementally.
    beginDraft(kind: AnnotationGeometry["kind"], point: { x: number; y: number }) {
      const id = nextId();
      let geometry: AnnotationGeometry;
      switch (kind) {
        case "arrow":
          geometry = { kind: "arrow", tail: point, tip: point };
          break;
        case "rectangle":
          geometry = { kind: "rectangle", origin: point, size: { width: 0, height: 0 } };
          break;
        case "numbered_badge":
          geometry = { kind: "numbered_badge", center: point, radius: BADGE_RADIUS_PX };
          break;
        case "text":
          geometry = {
            kind: "text",
            origin: point,
            size: { width: 0, height: 0 },
            text: "",
          };
          break;
        case "blur":
          geometry = {
            kind: "blur",
            origin: point,
            size: { width: 0, height: 0 },
            radius: DEFAULT_BLUR_RADIUS,
          };
          break;
        default:
          // Exhaustiveness — TypeScript's discriminated union ensures
          // every variant is handled. Fallback to a rectangle so a
          // misconfigured tool still produces a visible shape rather
          // than panicking.
          geometry = { kind: "rectangle", origin: point, size: { width: 0, height: 0 } };
      }
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
      } else if (draft.geometry.kind === "text") {
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
      } else if (draft.geometry.kind === "blur") {
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

    /// Commit the text content of an in-flight text draft. Called by
    /// the overlay editor when the user presses Enter (no Shift) or
    /// clicks outside the overlay. Pushes the pre-mutation snapshot
    /// so the commit is undoable, then promotes the draft.
    /// An empty text payload is treated as a cancel.
    commitText(text: string): void {
      const draft = inner.draft;
      if (!draft || draft.geometry.kind !== "text") return;
      draft.geometry.text = text;
      if (isDraftDegenerate()) {
        inner.draft = null;
        return;
      }
      pushHistory();
      promoteDraft();
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

    // --- Selection -------------------------------------------------

    /// Replace the selection with a single annotation. `null` clears
    /// the selection (the empty-canvas click also routes here).
    selectOnly(id: number | null) {
      if (id === null) {
        inner.selection.clear();
        return;
      }
      // Selecting an annotation implicitly switches to the select
      // tool so the next pointermove does not start a fresh draw.
      if (inner.tool !== "select") {
        pushHistory();
        inner.tool = "select";
      }
      inner.selection.clear();
      inner.selection.add(id);
    },
    /// Add an annotation to the existing selection (Shift-click).
    selectAdd(id: number) {
      inner.selection.add(id);
    },
    /// Remove an annotation from the selection.
    selectRemove(id: number) {
      inner.selection.delete(id);
    },
    /// Toggle a single annotation's selection membership.
    selectToggle(id: number) {
      if (inner.selection.has(id)) {
        inner.selection.delete(id);
      } else {
        inner.selection.add(id);
      }
    },
    /// Extend / replace the selection with every annotation that
    /// intersects the marquee rect. `mode === "add"` is the
    /// Shift+drag semantic; `"replace"` is the bare drag.
    selectMarquee(rect: PhysicalRect, mode: "replace" | "add") {
      if (mode === "replace") {
        inner.selection.clear();
      }
      for (const ann of inner.annotations) {
        if (rectIntersects(annotationBounds(ann), rect)) {
          inner.selection.add(ann.id);
        }
      }
    },
    /// Select every annotation on the canvas.
    selectAll() {
      if (inner.annotations.length === 0) return;
      pushHistory();
      for (const ann of inner.annotations) {
        inner.selection.add(ann.id);
      }
    },
    clearSelection() {
      inner.selection.clear();
    },
    /// True iff the annotation is currently selected.
    isSelected(id: number): boolean {
      return inner.selection.has(id);
    },
    /// The axis-aligned bounding box of the current selection, or
    /// `null` when no annotation is selected. Empty selection
    /// returns `null` so the overlay does not render a stale
    /// rectangle.
    selectionBounds(): PhysicalRect | null {
      const anns = selectedAnnotations();
      if (anns.length === 0) return null;
      let minX = Infinity;
      let minY = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;
      for (const ann of anns) {
        const b = annotationBounds(ann);
        minX = Math.min(minX, b.origin.x);
        minY = Math.min(minY, b.origin.y);
        maxX = Math.max(maxX, b.origin.x + b.size.width);
        maxY = Math.max(maxY, b.origin.y + b.size.height);
      }
      return {
        origin: { x: minX, y: minY },
        size: { width: Math.max(0, maxX - minX), height: Math.max(0, maxY - minY) },
      };
    },
    /// Toolbar driver: the unified colour state for the current
    /// selection. `null` ⇒ no selection (toolbar falls back to the
    /// next-draw colour); `"mixed"` ⇒ heterogeneous selection; a
    /// concrete colour ⇒ every selected annotation matches.
    selectionColor(): SelectionColorState {
      const anns = selectedAnnotations();
      if (anns.length === 0) return null;
      const first = anns[0].color;
      for (const ann of anns) {
        if (ann.color !== first) return "mixed";
      }
      return first;
    },
    /// Toolbar driver: the unified stroke state for the selection.
    /// Mirror of `selectionColor()`.
    selectionStroke(): SelectionStrokeState {
      const anns = selectedAnnotations();
      if (anns.length === 0) return null;
      const first = anns[0].stroke;
      for (const ann of anns) {
        if (ann.stroke !== first) return "mixed";
      }
      return first;
    },

    // --- Per-geometry handle contract -------------------------------

    /// The set of handles a geometry exposes to the overlay. The
    /// overlay renders exactly these handles so the user never sees a
    /// "resize" handle on a badge or a "wrap" handle on a rectangle.
    /// `move` is always included so a body-drag is consistent across
    /// every annotation type.
    handlesFor(annotation: Annotation): TransformHandle[] {
      const g = annotation.geometry;
      switch (g.kind) {
        case "arrow":
          return ["move", "tail", "tip"];
        case "rectangle":
          return ["move", "nw", "n", "ne", "e", "se", "s", "sw", "w"];
        case "text":
          // Vertical extent is fixed by text content; only horizontal
          // resize applies. The left/right handles adjust the box
          // width without touching the text baseline.
          return ["move", "left", "right"];
        case "numbered_badge":
          // Rigid badge: translate-only. The radius is a fixed
          // session constant.
          return ["move"];
        case "blur":
          return ["move", "nw", "n", "ne", "e", "se", "s", "sw", "w"];
      }
    },

    // --- Transform gestures ----------------------------------------

    /// Start a per-annotation transform at the given handle. Snapshots
    /// the pre-mutation state so the gesture is one undoable action.
    /// Refuses to start when a transform is already in flight; the
    /// overlay is expected to commit the previous gesture before
    /// starting a new one.
    beginTransform(id: number, handle: TransformHandle, cursor: PhysicalPoint) {
      const ann = findById(id);
      if (!ann) return;
      if (inner.transform) return;
      pushHistory();
      inner.transform = {
        kind: "transform",
        snapshot: snapshotOf(inner),
        ids: [id],
        handle,
        startCursor: clampPoint(cursor),
        // Deep-copy via the snapshot path so a stray mutation doesn't
        // leak into the snapshot.
        initialGeometry: $state.snapshot(ann.geometry) as AnnotationGeometry,
        initialGeometryMap: null,
      };
    },
    /// Apply a pointer-move frame to the in-flight transform. The
    /// projection is computed from the snapshotted initial geometry
    /// (captured at gesture start) so the result depends only on the
    /// current cursor and the start position — never on prior frames.
    updateTransform(cursor: PhysicalPoint) {
      const t = inner.transform;
      if (!t || t.kind !== "transform" || t.ids.length === 0) return;
      const id = t.ids[0];
      const ann = findById(id);
      if (!ann || !t.initialGeometry) return;
      const c = clampPoint(cursor);
      const dx = c.x - t.startCursor.x;
      const dy = c.y - t.startCursor.y;
      const g = t.initialGeometry;
      const handle = t.handle;
      if (g.kind === "arrow") {
        if (handle === "tail") {
          ann.geometry = { kind: "arrow", tail: c, tip: g.tip };
        } else if (handle === "tip") {
          ann.geometry = { kind: "arrow", tail: g.tail, tip: c };
        } else if (handle === "move") {
          ann.geometry = {
            kind: "arrow",
            tail: { x: g.tail.x + dx, y: g.tail.y + dy },
            tip: { x: g.tip.x + dx, y: g.tip.y + dy },
          };
        }
        return;
      }
      if (g.kind === "rectangle" || g.kind === "blur") {
        const origin = { ...g.origin };
        const size = { ...g.size };
        if (handle === "move") {
          ann.geometry = {
            ...g,
            origin: { x: g.origin.x + dx, y: g.origin.y + dy },
          } as AnnotationGeometry;
          return;
        }
        if (handle) {
          apply8Handle(origin, size, handle, c);
        }
        ann.geometry = {
          ...g,
          origin,
          size: {
            width: Math.max(MIN_BOX_DIMENSION, size.width),
            height: Math.max(MIN_BOX_DIMENSION, size.height),
          },
        } as AnnotationGeometry;
        return;
      }
      if (g.kind === "text") {
        if (handle === "move") {
          ann.geometry = {
            ...g,
            origin: { x: g.origin.x + dx, y: g.origin.y + dy },
          };
          return;
        }
        // Horizontal-only resize. The vertical extent is determined
        // by the text content at flatten time, so vertical handles
        // are not exposed.
        const origin = { ...g.origin };
        const size = { ...g.size };
        if (handle === "left") {
          const newX = Math.min(c.x, g.origin.x + g.size.width - MIN_BOX_DIMENSION);
          const newWidth = g.origin.x + g.size.width - newX;
          origin.x = newX;
          size.width = Math.max(MIN_BOX_DIMENSION, newWidth);
        } else if (handle === "right") {
          size.width = Math.max(MIN_BOX_DIMENSION, c.x - g.origin.x);
        }
        ann.geometry = { ...g, origin, size };
        return;
      }
      if (g.kind === "numbered_badge") {
        if (handle === "move") {
          ann.geometry = {
            ...g,
            center: { x: g.center.x + dx, y: g.center.y + dy },
          };
        }
      }
    },
    /// Commit the in-flight transform. The pre-mutation snapshot was
    /// pushed at `beginTransform` time, so the gesture already owns
    /// one history entry — `endTransform` is a no-op for history. It
    /// clears the active state so the next gesture can begin.
    endTransform() {
      inner.transform = null;
    },
    /// Abort the in-flight transform. The pre-mutation snapshot was
    /// pushed at `beginTransform` and must be discarded so the user
    /// can undo back to the previous state without a phantom entry.
    /// We undo the gesture in place by restoring the snapshot.
    cancelTransform() {
      const t = inner.transform;
      if (!t) return;
      // The snapshot was pushed at gesture start; pop it without
      // touching `future` (Escape is intentionally not a redoable
      // action).
      inner.history.past.pop();
      restoreSnapshot(t.snapshot);
      inner.transform = null;
    },

    /// Start a multi-select translate gesture. The gesture is one
    /// undoable action regardless of how many annotations are in the
    /// selection; the ids are captured at start so a late join does
    /// not tag along. The initial geometry of every selected
    /// annotation is snapshotted so frame-to-frame mutations are
    /// projected from the original (no drift).
    beginTranslateSelection(cursor: PhysicalPoint) {
      if (inner.transform) return;
      if (inner.selection.size === 0) return;
      const map = new Map<number, AnnotationGeometry>();
      for (const id of inner.selection) {
        const ann = findById(id);
        if (ann) {
          map.set(id, $state.snapshot(ann.geometry) as AnnotationGeometry);
        }
      }
      if (map.size === 0) return;
      pushHistory();
      inner.transform = {
        kind: "translate",
        snapshot: snapshotOf(inner),
        ids: Array.from(map.keys()),
        startCursor: clampPoint(cursor),
        initialGeometry: null,
        initialGeometryMap: map,
      };
    },
    /// Apply a frame to the translate gesture. Every annotation is
    /// moved by the full delta from the gesture start — never the
    /// frame-to-frame delta — so the gesture is drift-free.
    updateTranslateSelection(cursor: PhysicalPoint) {
      const t = inner.transform;
      if (!t || t.kind !== "translate" || !t.initialGeometryMap) return;
      const c = clampPoint(cursor);
      const dx = c.x - t.startCursor.x;
      const dy = c.y - t.startCursor.y;
      for (const id of t.ids) {
        const ann = findById(id);
        if (!ann) continue;
        const g = t.initialGeometryMap.get(id);
        if (!g) continue;
        if (g.kind === "arrow") {
          ann.geometry = {
            kind: "arrow",
            tail: { x: g.tail.x + dx, y: g.tail.y + dy },
            tip: { x: g.tip.x + dx, y: g.tip.y + dy },
          };
        } else if (g.kind === "rectangle" || g.kind === "text" || g.kind === "blur") {
          ann.geometry = {
            ...g,
            origin: { x: g.origin.x + dx, y: g.origin.y + dy },
          };
        } else if (g.kind === "numbered_badge") {
          ann.geometry = {
            ...g,
            center: { x: g.center.x + dx, y: g.center.y + dy },
          };
        }
      }
    },
    endTranslateSelection() {
      inner.transform = null;
    },

    // --- Batch style ------------------------------------------------

    /// Allow-list of geometry kinds that participate in colour
    /// edits. Every visible primitive updates its `color`; blur
    /// ignores the field at flatten time but is kept in the
    /// allow-list so a "blur + arrow" selection produces a reversible
    /// batch update on both (the arrow's colour visibly changes; the
    /// blur's stored colour updates for the next non-blur reuse).
    applyColorToSelection(color: AnnotationColor) {
      const anns = selectedAnnotations();
      if (anns.length === 0) {
        // Empty selection: fall back to the next-draw style so the
        // toolbar shortcut behaves the same with or without a
        // selection.
        pushHistory();
        inner.color = color;
        return;
      }
      pushHistory();
      for (const ann of anns) {
        ann.color = color;
      }
    },
    /// Mirror of `applyColorToSelection` for stroke width.
    applyStrokeToSelection(stroke: AnnotationStroke) {
      const anns = selectedAnnotations();
      if (anns.length === 0) {
        pushHistory();
        inner.stroke = stroke;
        return;
      }
      pushHistory();
      for (const ann of anns) {
        ann.stroke = stroke;
      }
    },

    // --- Deletion + z-order ----------------------------------------

    /// Remove every selected annotation. The undo restores the
    /// original list so a bulk delete is reversible.
    deleteSelection() {
      if (inner.selection.size === 0) return;
      pushHistory();
      const ids = inner.selection;
      inner.annotations = inner.annotations.filter((a) => !ids.has(a.id));
      inner.selection.clear();
    },
    /// Raise the entire selection by one z-order slot. The topmost
    /// selected annotation stays clamped at the current max.
    raiseSelection() {
      const anns = selectedAnnotations();
      if (anns.length === 0) return;
      pushHistory();
      for (const ann of anns) {
        ann.zOrder += 1;
      }
    },
    /// Lower the entire selection by one z-order slot. The bottommost
    /// selected annotation stays clamped at the current min.
    lowerSelection() {
      const anns = selectedAnnotations();
      if (anns.length === 0) return;
      pushHistory();
      for (const ann of anns) {
        ann.zOrder -= 1;
      }
    },
    /// Bring the entire selection to the front of the z-stack.
    bringToFrontSelection() {
      const anns = selectedAnnotations();
      if (anns.length === 0) return;
      pushHistory();
      const maxZ = inner.annotations.reduce((m, a) => Math.max(m, a.zOrder), 0);
      let cursor = maxZ + 1;
      for (const ann of anns) {
        ann.zOrder = cursor;
        cursor += 1;
      }
    },
    /// Send the entire selection to the back of the z-stack.
    sendToBackSelection() {
      const anns = selectedAnnotations();
      if (anns.length === 0) return;
      pushHistory();
      const minZ = inner.annotations.reduce((m, a) => Math.min(m, a.zOrder), 0);
      let cursor = minZ - 1;
      for (const ann of anns) {
        ann.zOrder = cursor;
        cursor -= 1;
      }
    },

    // --- Undo / redo -----------------------------------------------

    /// Undo the most recent committed action. Returns true on success.
    /// Restores the annotation list, the active tool, the colour, the
    /// stroke, and the selection set from the popped snapshot. The
    /// badge counter is session-scoped and is intentionally NOT
    /// restored.
    undo(): boolean {
      const past = inner.history.past;
      if (past.length === 0) return false;
      const previous = past.pop()!;
      inner.history.future.push(snapshotOf(inner));
      restoreSnapshot(previous);
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
      restoreSnapshot(next);
      inner.draft = null;
      return true;
    },

    /// Reset every store field. Called on session cleanup so a fresh
    /// session starts with no annotations, no badge counter carry,
    /// no selection, and no history.
    reset(): void {
      // Drop any in-flight transform so a reset between sessions
      // cannot leak through a forgotten Escape.
      inner.transform = null;
      Object.assign(inner, defaultState());
    },

    /// Discard annotations when the user clears the crop and starts a new
    /// crop within the same capture. Tool and style choices remain useful,
    /// but the old scene, badge numbering, and undo branch cannot leak into
    /// a different exported rectangle.
    clearForRecrop(): void {
      inner.annotations = [];
      inner.draft = null;
      inner.selection.clear();
      inner.history = { past: [], future: [] };
      inner.transform = null;
      inner.badgeCounter = 1;
    },
  };
}

/// Single store instance shared by every overlay view. The store is
/// created lazily so test files can `reset()` it between scenarios
/// without touching global state.
export const annotationStore = createAnnotationStore();

/**
 * Parse a clamped 8-handle drag into a new origin + size. The
 * caller supplies the initial rect and the resolved handle + cursor
 * position; the function computes the new origin/size without
 * mutating the inputs. Used by `updateTransform` for both
 * rectangles and blur regions (the geometry is structurally
 * identical: an axis-aligned box with a fixed top-left origin).
 */
function apply8Handle(
  origin: PhysicalPoint,
  size: PhysicalSize,
  handle: TransformHandle,
  cursor: PhysicalPoint,
): void {
  const { x: x0, y: y0, width: w0, height: h0 } = { x: origin.x, y: origin.y, ...size };
  const x1 = x0 + w0;
  const y1 = y0 + h0;
  switch (handle) {
    case "nw":
      origin.x = Math.min(cursor.x, x1 - MIN_BOX_DIMENSION);
      origin.y = Math.min(cursor.y, y1 - MIN_BOX_DIMENSION);
      size.width = Math.max(MIN_BOX_DIMENSION, x1 - origin.x);
      size.height = Math.max(MIN_BOX_DIMENSION, y1 - origin.y);
      return;
    case "n":
      origin.y = Math.min(cursor.y, y1 - MIN_BOX_DIMENSION);
      size.height = Math.max(MIN_BOX_DIMENSION, y1 - origin.y);
      return;
    case "ne":
      origin.y = Math.min(cursor.y, y1 - MIN_BOX_DIMENSION);
      size.width = Math.max(MIN_BOX_DIMENSION, cursor.x - x0);
      size.height = Math.max(MIN_BOX_DIMENSION, y1 - origin.y);
      return;
    case "e":
      size.width = Math.max(MIN_BOX_DIMENSION, cursor.x - x0);
      return;
    case "se":
      size.width = Math.max(MIN_BOX_DIMENSION, cursor.x - x0);
      size.height = Math.max(MIN_BOX_DIMENSION, cursor.y - y0);
      return;
    case "s":
      size.height = Math.max(MIN_BOX_DIMENSION, cursor.y - y0);
      return;
    case "sw":
      origin.x = Math.min(cursor.x, x1 - MIN_BOX_DIMENSION);
      size.width = Math.max(MIN_BOX_DIMENSION, x1 - origin.x);
      size.height = Math.max(MIN_BOX_DIMENSION, cursor.y - y0);
      return;
    case "w":
      origin.x = Math.min(cursor.x, x1 - MIN_BOX_DIMENSION);
      size.width = Math.max(MIN_BOX_DIMENSION, x1 - origin.x);
      return;
    default:
      // `move` is handled by the caller because it requires the
      // gesture delta, not the cursor position.
      return;
  }
}
