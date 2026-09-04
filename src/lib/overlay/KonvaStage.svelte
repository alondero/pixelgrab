<script lang="ts">
  // Konva-driven overlay stage. Hosts three layered UI surfaces:
  //   1. The frozen-frame image + the dim mask / crosshair / region
  //      selection rectangle (handles the user's crop — tracer-02).
  //   2. The annotation layer (tracer-04): renders every committed
  //      annotation plus the in-flight draft, captures pointer events
  //      for the drawing tools.
  //   3. The annotation-selection chrome (tracer-06): per-annotation
  //      bounding boxes + resolved handles, plus the marquee rectangle
  //      for shift-drag group selection.
  //
  // The three surfaces share a single pointer pipeline. When the
  // active tool is `select`, the annotation pipeline takes the pointer
  // (click on an annotation selects it; shift-click toggles; bare
  // drag creates a marquee). When the tool is arrow/rectangle/etc.,
  // the annotation pipeline owns the pointer for a draw gesture.

  import { onMount } from "svelte";
  import Konva from "konva";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Annotation, PhysicalBounds, PhysicalPoint } from "$lib/ipc/types";
  import { stageToPhysicalPoint } from "./coordinates";
  import { annotationStore, type TransformHandle } from "$lib/annotation/store.svelte";
  import type { AnnotationColor, AnnotationStroke } from "$lib/ipc/types";

  // Issue #63: the capture asset may be a local file path (bounded
  // transport) rather than an inline data URL. Local paths load via
  // the Tauri asset protocol; data URLs pass through unchanged.
  function resolveAssetUrl(url: string): string {
    if (url.startsWith("data:") || url.startsWith("asset:") || url.startsWith("http")) {
      return url;
    }
    return convertFileSrc(url);
  }

  interface Props {
    assetUrl: string;
    bounds: PhysicalBounds;
    stageWidth: number;
    stageHeight: number;
    onSelectionChange: (bounds: PhysicalBounds | null) => void;
    onCommit?: (target?: "shelf" | "clipboard") => void;
    onCancel?: () => void;
    onSaveAs?: () => void;
  }

  let {
    assetUrl,
    bounds,
    stageWidth,
    stageHeight,
    onSelectionChange,
    onCommit,
    onCancel,
    onSaveAs,
  }: Props = $props();

  let container: HTMLDivElement;
  let stage: Konva.Stage | null = null;
  let imageNode: Konva.Image | null = null;
  // Crop selection (tracer-02).
  let dimMaskTop: Konva.Rect | null = null;
  let dimMaskBottom: Konva.Rect | null = null;
  let dimMaskLeft: Konva.Rect | null = null;
  let dimMaskRight: Konva.Rect | null = null;
  let crosshairH: Konva.Line | null = null;
  let crosshairV: Konva.Line | null = null;
  let cropSelectionRect: Konva.Rect | null = null;
  let cropSelectionBorder: Konva.Rect | null = null;
  let cropHandles: Konva.Rect[] = [];
  // Annotation layer.
  let annotationLayer: Konva.Layer | null = null;
  let annotationNodes = new Map<number, Konva.Group>();
  let draftNode: Konva.Group | null = null;
  // Annotation selection chrome (tracer-06).
  let annotationSelectionLayer: Konva.Layer | null = null;
  let selectionBoxNodes = new Map<number, Konva.Rect>();
  let selectionHandleNodes = new Map<string, Konva.Rect>();
  let marqueeRect: Konva.Rect | null = null;
  // Region selection state (tracer-02).
  let dragging = $state(false);
  let startPoint: { x: number; y: number } | null = null;
  let pointerPos = $state<{ x: number; y: number } | null>(null);
  let activeCropHandle: HandlePosition | null = null;
  let lastSelection = $state<PhysicalBounds | null>(null);
  // Region-lock flag: once a selection is committed, drawing tools
  // take over the pointer until the user cancels or commits.
  let drawingDraft = $state(false);
  let draftStart: PhysicalPoint | null = null;
  // Annotation selection gesture state (tracer-06).
  let marqueeDragging = $state(false);
  let marqueeStart: { x: number; y: number } | null = null;
  let annotationDragging = $state(false);
  // Text editor overlay visibility. Set true at the end of a text
  // draft drag; reset when the overlay commits or cancels.
  let textEditing = $state(false);
  // IME composition flag. When `true`, the editor is mid-CJK or
  // candidate selection; Enter / Escape are suppressed so the IME's
  // own confirmation Enter does not commit prematurely.
  let isComposing = $state(false);

  type HandlePosition = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

  const CROP_HANDLE_SIZE = 10;
  const ANNOTATION_HANDLE_SIZE = 8;
  const MIN_SELECTION_DIM = 4;

  // Palette mirror: must agree with `crates/pixelgrab-contracts/src/annotation.rs`
  // `AnnotationColor::rgba` so the Konva preview and the rasterized
  // export use the same hue.
  const COLOR_HEX: Record<AnnotationColor, string> = {
    red: "#e53b3b",
    green: "#3be55c",
    blue: "#3b82e5",
    yellow: "#f6e33b",
    white: "#ffffff",
  };

  const STROKE_PX: Record<AnnotationStroke, number> = {
    thin: 2,
    medium: 4,
    thick: 8,
  };

  function clampPositive(value: number) {
    return Math.max(0, Math.round(value));
  }

  function emitPhysicalSelection(
    rect: { x: number; y: number; width: number; height: number } | null,
  ) {
    if (!rect) {
      lastSelection = null;
      onSelectionChange(null);
      return;
    }
    const scaleX = bounds.size.width / stageWidth;
    const scaleY = bounds.size.height / stageHeight;
    const physical: PhysicalBounds = {
      origin: {
        x: bounds.origin.x + Math.round(rect.x * scaleX),
        y: bounds.origin.y + Math.round(rect.y * scaleY),
      },
      size: {
        width: Math.round(rect.width * scaleX),
        height: Math.round(rect.height * scaleY),
      },
    };
    if (physical.size.width < 4 || physical.size.height < 4) {
      lastSelection = null;
      onSelectionChange(null);
      return;
    }
    lastSelection = physical;
    onSelectionChange(physical);
  }

  /// Convert a CSS-pixel pointer position inside the stage to a
  /// physical-pixel position inside the active crop. Returns `null`
  /// when the pointer is outside the crop rectangle (so a draft cannot
  /// be created off-canvas).
  function pointerToCropLocal(
    pos: { x: number; y: number },
    crop: { x: number; y: number; width: number; height: number },
  ): PhysicalPoint | null {
    if (
      pos.x < crop.x ||
      pos.x > crop.x + crop.width ||
      pos.y < crop.y ||
      pos.y > crop.y + crop.height
    ) {
      return null;
    }
    const local = stageToPhysicalPoint({ x: pos.x - crop.x, y: pos.y - crop.y }, bounds.size, {
      width: stageWidth,
      height: stageHeight,
    });
    return { x: clampPositive(local.x), y: clampPositive(local.y) };
  }

  function cropCssRect(): { x: number; y: number; width: number; height: number } | null {
    if (!lastSelection) return null;
    const scaleX = stageWidth / bounds.size.width;
    const scaleY = stageHeight / bounds.size.height;
    return {
      x: (lastSelection.origin.x - bounds.origin.x) * scaleX,
      y: (lastSelection.origin.y - bounds.origin.y) * scaleY,
      width: lastSelection.size.width * scaleX,
      height: lastSelection.size.height * scaleY,
    };
  }

  function updateDimMask(rect: { x: number; y: number; width: number; height: number } | null) {
    if (!dimMaskTop || !dimMaskBottom || !dimMaskLeft || !dimMaskRight || !stage) {
      return;
    }
    if (!rect) {
      dimMaskTop.visible(false);
      dimMaskBottom.visible(false);
      dimMaskLeft.visible(false);
      dimMaskRight.visible(false);
      return;
    }
    const w = stage.width();
    const h = stage.height();
    const right = rect.x + rect.width;
    const bottom = rect.y + rect.height;
    dimMaskTop.position({ x: 0, y: 0 });
    dimMaskTop.size({ width: w, height: clampPositive(rect.y) });
    dimMaskTop.visible(true);

    dimMaskBottom.position({ x: 0, y: bottom });
    dimMaskBottom.size({
      width: w,
      height: clampPositive(h - bottom),
    });
    dimMaskBottom.visible(true);

    dimMaskLeft.position({ x: 0, y: rect.y });
    dimMaskLeft.size({
      width: clampPositive(rect.x),
      height: clampPositive(rect.height),
    });
    dimMaskLeft.visible(true);

    dimMaskRight.position({ x: right, y: rect.y });
    dimMaskRight.size({
      width: clampPositive(w - right),
      height: clampPositive(rect.height),
    });
    dimMaskRight.visible(true);
  }

  function positionCropHandles(
    rect: { x: number; y: number; width: number; height: number } | null,
  ) {
    if (cropHandles.length !== 8 || !rect) {
      for (const handle of cropHandles) handle.visible(false);
      return;
    }
    const positions: Array<{ x: number; y: number }> = [
      { x: rect.x - CROP_HANDLE_SIZE / 2, y: rect.y - CROP_HANDLE_SIZE / 2 },
      { x: rect.x + rect.width / 2 - CROP_HANDLE_SIZE / 2, y: rect.y - CROP_HANDLE_SIZE / 2 },
      { x: rect.x + rect.width - CROP_HANDLE_SIZE / 2, y: rect.y - CROP_HANDLE_SIZE / 2 },
      {
        x: rect.x + rect.width - CROP_HANDLE_SIZE / 2,
        y: rect.y + rect.height / 2 - CROP_HANDLE_SIZE / 2,
      },
      {
        x: rect.x + rect.width - CROP_HANDLE_SIZE / 2,
        y: rect.y + rect.height - CROP_HANDLE_SIZE / 2,
      },
      {
        x: rect.x + rect.width / 2 - CROP_HANDLE_SIZE / 2,
        y: rect.y + rect.height - CROP_HANDLE_SIZE / 2,
      },
      { x: rect.x - CROP_HANDLE_SIZE / 2, y: rect.y + rect.height - CROP_HANDLE_SIZE / 2 },
      { x: rect.x - CROP_HANDLE_SIZE / 2, y: rect.y + rect.height / 2 - CROP_HANDLE_SIZE / 2 },
    ];
    positions.forEach((pos, index) => {
      const handle = cropHandles[index];
      handle.position(pos);
      handle.size({ width: CROP_HANDLE_SIZE, height: CROP_HANDLE_SIZE });
      handle.visible(true);
    });
  }

  function cropHandleHit(
    pos: { x: number; y: number },
    rect: { x: number; y: number; width: number; height: number },
  ): HandlePosition | null {
    const positions: Array<[HandlePosition, number, number]> = [
      ["nw", rect.x, rect.y],
      ["n", rect.x + rect.width / 2, rect.y],
      ["ne", rect.x + rect.width, rect.y],
      ["e", rect.x + rect.width, rect.y + rect.height / 2],
      ["se", rect.x + rect.width, rect.y + rect.height],
      ["s", rect.x + rect.width / 2, rect.y + rect.height],
      ["sw", rect.x, rect.y + rect.height],
      ["w", rect.x, rect.y + rect.height / 2],
    ];
    const tolerance = CROP_HANDLE_SIZE;
    for (const [name, hx, hy] of positions) {
      if (Math.abs(pos.x - hx) <= tolerance && Math.abs(pos.y - hy) <= tolerance) {
        return name;
      }
    }
    return null;
  }

  function applyCropHandleDrag(
    startRect: { x: number; y: number; width: number; height: number },
    handle: HandlePosition,
    pos: { x: number; y: number },
  ) {
    let x = startRect.x;
    let y = startRect.y;
    let width = startRect.width;
    let height = startRect.height;
    switch (handle) {
      case "nw":
        width = startRect.width + (startRect.x - pos.x);
        height = startRect.height + (startRect.y - pos.y);
        x = pos.x;
        y = pos.y;
        break;
      case "n":
        height = startRect.height + (startRect.y - pos.y);
        y = pos.y;
        break;
      case "ne":
        width = pos.x - startRect.x;
        height = startRect.height + (startRect.y - pos.y);
        y = pos.y;
        break;
      case "e":
        width = pos.x - startRect.x;
        break;
      case "se":
        width = pos.x - startRect.x;
        height = pos.y - startRect.y;
        break;
      case "s":
        height = pos.y - startRect.y;
        break;
      case "sw":
        width = startRect.width + (startRect.x - pos.x);
        height = pos.y - startRect.y;
        x = pos.x;
        break;
      case "w":
        width = startRect.width + (startRect.x - pos.x);
        x = pos.x;
        break;
    }
    width = Math.max(4, width);
    height = Math.max(4, height);
    if (x < 0) {
      width += x;
      x = 0;
    }
    if (y < 0) {
      height += y;
      y = 0;
    }
    return { x, y, width, height };
  }

  function cropSelectionGeometry(): { x: number; y: number; width: number; height: number } | null {
    if (!cropSelectionRect) return null;
    return {
      x: cropSelectionRect.x(),
      y: cropSelectionRect.y(),
      width: cropSelectionRect.width(),
      height: cropSelectionRect.height(),
    };
  }

  function redrawCropOverlay(rect: { x: number; y: number; width: number; height: number } | null) {
    if (!cropSelectionRect || !cropSelectionBorder || !stage) return;
    if (!rect) {
      cropSelectionRect.visible(false);
      cropSelectionBorder.visible(false);
      updateDimMask(null);
      positionCropHandles(null);
      return;
    }
    cropSelectionRect.position({ x: rect.x, y: rect.y });
    cropSelectionRect.size({ width: rect.width, height: rect.height });
    cropSelectionRect.visible(true);
    cropSelectionBorder.position({ x: rect.x, y: rect.y });
    cropSelectionBorder.size({ width: rect.width, height: rect.height });
    cropSelectionBorder.visible(true);
    updateDimMask(rect);
    positionCropHandles(rect);
  }

  function updateCrosshair() {
    if (!crosshairH || !crosshairV || !stage || !pointerPos) {
      crosshairH?.visible(false);
      crosshairV?.visible(false);
      return;
    }
    crosshairH.points([0, pointerPos.y, stage.width(), pointerPos.y]);
    crosshairV.points([pointerPos.x, 0, pointerPos.x, stage.height()]);
    crosshairH.visible(true);
    crosshairV.visible(true);
  }

  // ---- Annotation rendering ----------------------------------------

  /// Map an annotation to a Konva node tree. The geometry lives in
  /// crop-local physical pixels; the layer translates by the crop's
  /// CSS offset so the drawn shape aligns with the user's pointer.
  function annotationNode(annotation: Annotation, isDraft: boolean): Konva.Group {
    const strokeWidth = STROKE_PX[annotation.stroke];
    const color = COLOR_HEX[annotation.color];
    const opacity = isDraft ? 0.6 : 1;
    const listening = !isDraft;
    if (annotation.geometry.kind === "arrow") {
      const group = new Konva.Group({ listening, opacity });
      const { tail, tip } = annotation.geometry;
      group.add(
        new Konva.Line({
          points: [tail.x, tail.y, tip.x, tip.y],
          stroke: color,
          strokeWidth,
          lineCap: "round",
          lineJoin: "round",
          listening: false,
        }),
      );
      // Head triangle.
      const dx = tail.x - tip.x;
      const dy = tail.y - tip.y;
      const len = Math.hypot(dx, dy);
      if (len > 0) {
        const ux = dx / len;
        const uy = dy / len;
        const headLen = Math.min(strokeWidth * 4, 32);
        const halfWidth = headLen * 0.6;
        const baseX = tip.x + ux * headLen;
        const baseY = tip.y + uy * headLen;
        const perpX = -uy;
        const perpY = ux;
        group.add(
          new Konva.Line({
            points: [
              tip.x,
              tip.y,
              baseX + perpX * halfWidth,
              baseY + perpY * halfWidth,
              baseX - perpX * halfWidth,
              baseY - perpY * halfWidth,
            ],
            closed: true,
            fill: color,
            stroke: color,
            strokeWidth: 1,
            listening: false,
          }),
        );
      }
      // The hit area is a thick stroked line so a thin arrow is
      // still clickable without ballooning the visible stroke.
      group.add(
        new Konva.Line({
          points: [tail.x, tail.y, tip.x, tip.y],
          stroke: "rgba(0,0,0,0.001)",
          strokeWidth: Math.max(strokeWidth + 8, 12),
          listening: true,
        }),
      );
      return group;
    }
    if (annotation.geometry.kind === "rectangle") {
      const { origin, size } = annotation.geometry;
      const group = new Konva.Group({ listening, opacity });
      group.add(
        new Konva.Rect({
          x: origin.x,
          y: origin.y,
          width: size.width,
          height: size.height,
          stroke: color,
          strokeWidth,
          listening: false,
        }),
      );
      // Invisible hit area for the body-drag.
      group.add(
        new Konva.Rect({
          x: origin.x,
          y: origin.y,
          width: Math.max(size.width, 4),
          height: Math.max(size.height, 4),
          fill: "rgba(0,0,0,0.001)",
          listening: true,
        }),
      );
      return group;
    }
    if (annotation.geometry.kind === "numbered_badge") {
      const { center, radius } = annotation.geometry;
      const group = new Konva.Group({ listening, opacity });
      group.add(
        new Konva.Circle({
          x: center.x,
          y: center.y,
          radius,
          fill: color,
          stroke: "#1a1a1a",
          strokeWidth: Math.max(1, strokeWidth / 2),
          listening: false,
        }),
      );
      if (annotation.number !== undefined) {
        const digits = String(annotation.number);
        group.add(
          new Konva.Text({
            x: center.x - radius,
            y: center.y - radius * 0.4,
            width: radius * 2,
            text: digits,
            fontSize: radius * 0.9,
            fontStyle: "bold",
            align: "center",
            fill: digitFillForColor(color),
            listening: false,
          }),
        );
      }
      // Hit area: a circle slightly larger than the visible badge.
      group.add(
        new Konva.Circle({
          x: center.x,
          y: center.y,
          radius: radius + 4,
          fill: "rgba(0,0,0,0.001)",
          listening: true,
        }),
      );
      return group;
    }
    if (annotation.geometry.kind === "text") {
      const { origin, size, text } = annotation.geometry;
      const group = new Konva.Group({ listening, opacity });
      group.add(
        new Konva.Rect({
          x: origin.x,
          y: origin.y,
          width: Math.max(size.width, 4),
          height: Math.max(size.height, 4),
          fill: "rgba(255, 255, 255, 0.85)",
          stroke: "#4f46e5",
          strokeWidth: 1,
          dash: [4, 3],
          listening: false,
        }),
      );
      if (text) {
        group.add(
          new Konva.Text({
            x: origin.x + 4,
            y: origin.y + 4,
            width: Math.max(size.width - 8, 0),
            height: Math.max(size.height - 8, 0),
            text,
            fontSize: 14,
            fontFamily: "system-ui, sans-serif",
            fill: "#141414",
            listening: false,
          }),
        );
      }
      group.add(
        new Konva.Rect({
          x: origin.x,
          y: origin.y,
          width: Math.max(size.width, 4),
          height: Math.max(size.height, 4),
          fill: "rgba(0,0,0,0.001)",
          listening: true,
        }),
      );
      return group;
    }
    if (annotation.geometry.kind === "blur") {
      const { origin, size } = annotation.geometry;
      const group = new Konva.Group({ listening, opacity });
      group.add(
        new Konva.Rect({
          x: origin.x,
          y: origin.y,
          width: Math.max(size.width, 4),
          height: Math.max(size.height, 4),
          fill: "rgba(0, 0, 0, 0.4)",
          stroke: "#4f46e5",
          strokeWidth: 1,
          dash: [4, 3],
          listening: false,
        }),
      );
      group.add(
        new Konva.Rect({
          x: origin.x,
          y: origin.y,
          width: Math.max(size.width, 4),
          height: Math.max(size.height, 4),
          fill: "rgba(0,0,0,0.001)",
          listening: true,
        }),
      );
      return group;
    }
    return new Konva.Group({ listening: false });
  }

  /// Compute the badge digit colour that contrasts with the badge
  /// fill. Mirrors `digit_color_for_luminance` in the Rust rasterizer
  /// so the Konva preview and the flattened PNG agree.
  function digitFillForColor(hex: string): string {
    const h = hex.replace("#", "");
    const r = parseInt(h.slice(0, 2), 16) / 255;
    const g = parseInt(h.slice(2, 4), 16) / 255;
    const b = parseInt(h.slice(4, 6), 16) / 255;
    const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    return lum < 0.5 ? "#ffffff" : "#141414";
  }

  /// Find the annotation under a CSS pointer position. The check uses
  /// the per-geometry hit rules (line-distance for arrows, rectangle
  /// containment for box geometries, distance for badges). Returns
  /// `null` when no annotation contains the pointer.
  function annotationHitTest(pos: { x: number; y: number }): Annotation | null {
    const crop = cropCssRect();
    if (!crop) return null;
    const local = pointerToCropLocal(pos, crop);
    if (!local) return null;
    // Iterate top-down so the highest-z-order annotation wins.
    const sorted = [...annotationStore.annotations].sort((a, b) => b.zOrder - a.zOrder);
    for (const ann of sorted) {
      if (annotationContainsLocal(ann, local)) return ann;
    }
    return null;
  }

  function annotationContainsLocal(ann: Annotation, p: PhysicalPoint): boolean {
    const g = ann.geometry;
    if (g.kind === "arrow") {
      return distanceToSegment(p, g.tail, g.tip) <= Math.max(8, STROKE_PX[ann.stroke] + 4);
    }
    if (g.kind === "rectangle" || g.kind === "text" || g.kind === "blur") {
      return (
        p.x >= g.origin.x &&
        p.x <= g.origin.x + g.size.width &&
        p.y >= g.origin.y &&
        p.y <= g.origin.y + g.size.height
      );
    }
    // Numbered badge: distance to the centre ≤ radius.
    const dx = p.x - g.center.x;
    const dy = p.y - g.center.y;
    return Math.hypot(dx, dy) <= g.radius + 4;
  }

  function distanceToSegment(p: PhysicalPoint, a: PhysicalPoint, b: PhysicalPoint): number {
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const len2 = dx * dx + dy * dy;
    if (len2 === 0) return Math.hypot(p.x - a.x, p.y - a.y);
    const t = Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2));
    const proj = { x: a.x + t * dx, y: a.y + t * dy };
    return Math.hypot(p.x - proj.x, p.y - proj.y);
  }

  function rerenderAnnotations() {
    if (!annotationLayer) return;
    annotationLayer.destroyChildren();
    annotationNodes = new Map();
    const crop = cropCssRect();
    if (crop) {
      annotationLayer.position({ x: crop.x, y: crop.y });
    } else {
      annotationLayer.position({ x: 0, y: 0 });
    }
    // Annotation geometry is stored in physical framebuffer pixels while
    // Konva draws in stage CSS pixels. Keep this conversion at the layer
    // boundary so arrows, boxes, badges, text, blur, and drafts all align.
    annotationLayer.scale({
      x: stageWidth / bounds.size.width,
      y: stageHeight / bounds.size.height,
    });
    for (const annotation of annotationStore.annotations) {
      const node = annotationNode(annotation, false);
      const id = annotation.id;
      // Attach the annotation id so a click handler can resolve the
      // underlying entity without a separate lookup.
      (node as unknown as { __annotationId?: number }).__annotationId = id;
      annotationLayer.add(node);
      annotationNodes.set(id, node);
    }
    const draft = annotationStore.draft;
    if (draft) {
      draftNode = annotationNode(draft, true);
      annotationLayer.add(draftNode);
    } else {
      draftNode = null;
    }
    annotationLayer.draw();
  }

  // Svelte action: focus the bound element on mount. Used by the
  // text-editor textarea so the keyboard lands on the editor
  // immediately after a drag-to-size commit. Avoids the
  // `a11y-autofocus` lint that the raw `autofocus` attribute
  // triggers.
  function focusOnMount(node: HTMLTextAreaElement) {
    node.focus();
    return {};
  }

  // ---- Annotation selection chrome (tracer-06) ---------------------

  /// Refresh the selection chrome (bounding box + handles) for every
  /// selected annotation. Called whenever the selection set, the
  /// annotations, or the crop rect changes.
  function rerenderSelection() {
    if (!annotationSelectionLayer) return;
    annotationSelectionLayer.destroyChildren();
    selectionBoxNodes = new Map();
    selectionHandleNodes = new Map();
    // `destroyChildren()` also destroys the marquee node created during
    // mount. Recreate it on every redraw or the first selection click makes
    // subsequent Shift-marquee gestures inert.
    marqueeRect = new Konva.Rect({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      stroke: "#4f46e5",
      strokeWidth: 1,
      dash: [3, 3],
      fill: "rgba(79, 70, 229, 0.08)",
      visible: false,
      listening: false,
    });
    annotationSelectionLayer.add(marqueeRect);
    const crop = cropCssRect();
    if (!crop) {
      annotationSelectionLayer.draw();
      return;
    }
    const scaleX = stageWidth / bounds.size.width;
    const scaleY = stageHeight / bounds.size.height;
    // Selection geometry shares the physical-pixel annotation coordinate
    // system. Scale at the layer boundary; the marquee is converted to
    // physical coordinates before it is drawn as well.
    annotationSelectionLayer.position({ x: crop.x, y: crop.y });
    annotationSelectionLayer.scale({ x: scaleX, y: scaleY });
    for (const ann of annotationStore.annotations) {
      if (!annotationStore.isSelected(ann.id)) continue;
      const rect = annotationBoundsLocal(ann);
      if (!rect) continue;
      const box = new Konva.Rect({
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
        stroke: "#4f46e5",
        strokeWidth: 1,
        dash: [4, 3],
        listening: false,
      });
      annotationSelectionLayer.add(box);
      selectionBoxNodes.set(ann.id, box);
      // Per-geometry handles.
      for (const handle of annotationStore.handlesFor(ann)) {
        const handleRect = createAnnotationHandle(ann.id, handle, rect, scaleX, scaleY);
        if (handleRect) {
          annotationSelectionLayer.add(handleRect);
          selectionHandleNodes.set(handleKey(ann.id, handle), handleRect);
        }
      }
    }
    annotationSelectionLayer.draw();
  }

  function handleKey(id: number, handle: TransformHandle): string {
    return `${id}:${handle}`;
  }

  function createAnnotationHandle(
    id: number,
    handle: TransformHandle,
    rect: { origin: { x: number; y: number }; size: { width: number; height: number } },
    scaleX: number,
    scaleY: number,
  ): Konva.Rect | null {
    if (handle === "move") return null; // The body itself is the move handle.
    let x: number;
    let y: number;
    if (handle === "tail") {
      // We don't know tail without geometry; the caller passes the
      // bounding rect, so use the closest corner as a stub for the
      // tail handle. The exact handle position is computed by
      // `annotationHandlePosition` below.
      return null;
    }
    if (handle === "tip") {
      return null;
    }
    if (handle === "left") {
      x = rect.origin.x;
      y = rect.origin.y + rect.size.height / 2;
    } else if (handle === "right") {
      x = rect.origin.x + rect.size.width;
      y = rect.origin.y + rect.size.height / 2;
    } else {
      const r = annotationHandlePosition(id, handle, rect);
      if (!r) return null;
      x = r.x;
      y = r.y;
    }
    // Convert from physical to CSS pixels so the handle size stays
    // constant on screen regardless of the crop scale.
    const sizeX = ANNOTATION_HANDLE_SIZE / scaleX;
    const sizeY = ANNOTATION_HANDLE_SIZE / scaleY;
    const handleRect = new Konva.Rect({
      x: x - sizeX / 2,
      y: y - sizeY / 2,
      width: sizeX,
      height: sizeY,
      fill: "white",
      stroke: "#4f46e5",
      strokeWidth: 1.5,
      listening: true,
    });
    (handleRect as unknown as { __annotationId?: number }).__annotationId = id;
    (handleRect as unknown as { __annotationHandle?: TransformHandle }).__annotationHandle = handle;
    return handleRect;
  }

  /// Compute the physical-pixel position of an annotation handle
  /// (excluding tail/tip, which are looked up directly from the
  /// geometry). Returns `null` for handles that don't apply to the
  /// bounding rect (e.g. `tail` geometry).
  function annotationHandlePosition(
    id: number,
    handle: TransformHandle,
    rect: { origin: { x: number; y: number }; size: { width: number; height: number } },
  ): { x: number; y: number } | null {
    const ann = annotationStore.annotations.find((a) => a.id === id);
    if (!ann) return null;
    const g = ann.geometry;
    if (g.kind === "arrow") {
      if (handle === "tail") return g.tail;
      if (handle === "tip") return g.tip;
      return null;
    }
    const { x, y, width: w, height: h } = { ...rect.origin, ...rect.size };
    switch (handle) {
      case "nw":
        return { x, y };
      case "n":
        return { x: x + w / 2, y };
      case "ne":
        return { x: x + w, y };
      case "e":
        return { x: x + w, y: y + h / 2 };
      case "se":
        return { x: x + w, y: y + h };
      case "s":
        return { x: x + w / 2, y: y + h };
      case "sw":
        return { x, y: y + h };
      case "w":
        return { x, y: y + h / 2 };
    }
    return null;
  }

  /// Compute the bounding box of an annotation in physical pixels.
  /// Mirrors the store's `annotationBounds` (kept duplicated here so
  /// the overlay can render the chrome without traversing the store
  /// on every refresh).
  function annotationBoundsLocal(ann: Annotation): {
    origin: { x: number; y: number };
    size: { width: number; height: number };
  } | null {
    const g = ann.geometry;
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
    return {
      origin: { x: g.center.x - g.radius, y: g.center.y - g.radius },
      size: { width: g.radius * 2, height: g.radius * 2 },
    };
  }

  /// Hit-test for the annotation selection chrome. Returns the
  /// `(id, handle)` of the matching handle, or `null` when none.
  function annotationHandleHitTest(pos: { x: number; y: number }): {
    id: number;
    handle: TransformHandle;
  } | null {
    const crop = cropCssRect();
    if (!crop) return null;
    const local = pointerToCropLocal(pos, crop);
    if (!local) return null;
    const scaleX = stageWidth / bounds.size.width;
    const scaleY = stageHeight / bounds.size.height;
    const toleranceX = ANNOTATION_HANDLE_SIZE / scaleX;
    const toleranceY = ANNOTATION_HANDLE_SIZE / scaleY;
    for (const ann of annotationStore.annotations) {
      if (!annotationStore.isSelected(ann.id)) continue;
      const rect = annotationBoundsLocal(ann);
      if (!rect) continue;
      for (const handle of annotationStore.handlesFor(ann)) {
        if (handle === "move") continue;
        const pos2 = annotationHandlePosition(ann.id, handle, rect);
        if (!pos2) continue;
        if (Math.abs(pos2.x - local.x) <= toleranceX && Math.abs(pos2.y - local.y) <= toleranceY) {
          return { id: ann.id, handle };
        }
      }
    }
    return null;
  }

  // ---- Pointer / keyboard handlers ---------------------------------

  function isDrawingTool(): boolean {
    return (
      annotationStore.tool === "arrow" ||
      annotationStore.tool === "rectangle" ||
      annotationStore.tool === "numbered_badge" ||
      annotationStore.tool === "text" ||
      annotationStore.tool === "blur"
    );
  }

  function isSelectTool(): boolean {
    return annotationStore.tool === "select";
  }

  function startDraft(pos: { x: number; y: number }) {
    if (!isDrawingTool()) return;
    const crop = cropCssRect();
    if (!crop) return;
    const local = pointerToCropLocal(pos, crop);
    if (!local) return;
    // Tracer-06: direct focus from drawing modes. If the user clicks
    // an existing annotation while a drawing tool is active, treat
    // the click as a select instead of starting a fresh draw.
    const hit = annotationHitTest(pos);
    if (hit) {
      annotationStore.selectOnly(hit.id);
      rerenderSelection();
      return;
    }
    drawingDraft = true;
    draftStart = local;
    const tool = annotationStore.tool;
    if (tool === "select") return;
    annotationStore.beginDraft(tool, local);
    rerenderAnnotations();
  }

  function continueDraft(pos: { x: number; y: number }) {
    if (!drawingDraft || !draftStart) return;
    const crop = cropCssRect();
    if (!crop) return;
    const local = pointerToCropLocal(pos, crop) ?? draftStart;
    annotationStore.updateDraft(local);
    rerenderAnnotations();
  }

  function endDraft() {
    if (!drawingDraft) return;
    drawingDraft = false;
    draftStart = null;
    const draft = annotationStore.draft;
    if (!draft) return;
    if (draft.geometry.kind === "arrow") {
      const dx = draft.geometry.tip.x - draft.geometry.tail.x;
      const dy = draft.geometry.tip.y - draft.geometry.tail.y;
      if (Math.hypot(dx, dy) < 4) {
        annotationStore.cancelDraft();
        rerenderAnnotations();
        return;
      }
    } else if (draft.geometry.kind === "rectangle") {
      if (draft.geometry.size.width < 4 || draft.geometry.size.height < 4) {
        annotationStore.cancelDraft();
        rerenderAnnotations();
        return;
      }
    } else if (draft.geometry.kind === "blur") {
      if (draft.geometry.size.width < 4 || draft.geometry.size.height < 4) {
        annotationStore.cancelDraft();
        rerenderAnnotations();
        return;
      }
    } else if (draft.geometry.kind === "text") {
      if (draft.geometry.size.width < 4 || draft.geometry.size.height < 4) {
        annotationStore.cancelDraft();
        rerenderAnnotations();
        return;
      }
      textEditing = true;
      rerenderAnnotations();
      return;
    }
    annotationStore.commitDraft();
    rerenderAnnotations();
  }

  function handleKeyDown(event: KeyboardEvent) {
    // Text editing owns Enter/Escape. Let the textarea handler process the
    // key instead of allowing the window-level commit shortcut to fire too.
    if (event.target instanceof HTMLTextAreaElement) return;
    const key = event.key.toLowerCase();
    if (event.ctrlKey || event.metaKey) {
      if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        annotationStore.undo();
        rerenderAnnotations();
        rerenderSelection();
        return;
      }
      if ((key === "z" && event.shiftKey) || key === "y") {
        event.preventDefault();
        annotationStore.redo();
        rerenderAnnotations();
        rerenderSelection();
        return;
      }
      if (key === "s") {
        event.preventDefault();
        onSaveAs?.();
        return;
      }
      if (key === "a") {
        event.preventDefault();
        annotationStore.selectAll();
        rerenderSelection();
        return;
      }
      // Tracer-06: z-order shortcuts. Ctrl+[ / Ctrl+] raise / lower;
      // Ctrl+Shift+[ / Ctrl+Shift+] bring to front / send to back.
      if (key === "[") {
        event.preventDefault();
        if (event.shiftKey) {
          annotationStore.sendToBackSelection();
        } else {
          annotationStore.lowerSelection();
        }
        rerenderAnnotations();
        return;
      }
      if (key === "]") {
        event.preventDefault();
        if (event.shiftKey) {
          annotationStore.bringToFrontSelection();
        } else {
          annotationStore.raiseSelection();
        }
        rerenderAnnotations();
        return;
      }
    }
    if (!event.ctrlKey && !event.metaKey && !event.altKey) {
      switch (key) {
        case "a":
          event.preventDefault();
          annotationStore.setTool("arrow");
          return;
        case "r":
          event.preventDefault();
          annotationStore.setTool("rectangle");
          return;
        case "n":
          event.preventDefault();
          annotationStore.setTool("numbered_badge");
          return;
        case "v":
          event.preventDefault();
          annotationStore.setTool("select");
          return;
        case "t":
          event.preventDefault();
          annotationStore.setTool("text");
          return;
        case "b":
          event.preventDefault();
          annotationStore.setTool("blur");
          return;
      }
      // Tracer-06: Delete / Backspace removes the selected set.
      if (event.key === "Delete" || event.key === "Backspace") {
        if (annotationStore.selection.size > 0) {
          event.preventDefault();
          annotationStore.deleteSelection();
          rerenderAnnotations();
          rerenderSelection();
        }
      }
    }
    // Enter publishes the normal shelf + clipboard result. Ctrl+C is the
    // fast clipboard-only path and must not leave a shelf card behind.
    if ((event.ctrlKey || event.metaKey) && key === "c") {
      if (lastSelection) {
        event.preventDefault();
        onCommit?.("clipboard");
      }
    } else if (event.key === "Enter" && lastSelection) {
      event.preventDefault();
      onCommit?.("shelf");
    }
    if (event.key === "Escape") {
      // First Escape: cancel an in-flight transform, drop the draft,
      // then clear the selection.
      if (annotationStore.transform) {
        event.preventDefault();
        annotationStore.cancelTransform();
        rerenderAnnotations();
        rerenderSelection();
        return;
      }
      if (annotationStore.draft || textEditing) {
        event.preventDefault();
        annotationStore.cancelDraft();
        textEditing = false;
        rerenderAnnotations();
        return;
      }
      if (annotationStore.selection.size > 0) {
        event.preventDefault();
        annotationStore.clearSelection();
        rerenderSelection();
        return;
      }
      // The crop lives in this component, not in the Rust orchestrator.
      // Clear it locally on the first Escape; only a second Escape with no
      // crop asks the backend to cancel and hide the capture session.
      if (lastSelection) {
        event.preventDefault();
        redrawCropOverlay(null);
        emitPhysicalSelection(null);
        rerenderSelection();
        return;
      }
      event.preventDefault();
      onCancel?.();
    }
  }

  onMount(() => {
    stage = new Konva.Stage({
      container,
      width: stageWidth,
      height: stageHeight,
    });

    const imageLayer = new Konva.Layer();
    const overlayLayer = new Konva.Layer();
    annotationLayer = new Konva.Layer({ listening: true });
    annotationSelectionLayer = new Konva.Layer({ listening: true });
    stage.add(imageLayer);
    stage.add(overlayLayer);
    stage.add(annotationLayer);
    stage.add(annotationSelectionLayer);

    const img = new Image();
    img.onload = () => {
      imageNode = new Konva.Image({
        image: img,
        x: 0,
        y: 0,
        width: stage!.width(),
        height: stage!.height(),
      });
      imageLayer.add(imageNode);
      imageLayer.draw();
    };
    img.src = resolveAssetUrl(assetUrl);

    const dimAttrs = {
      fill: "rgba(0, 0, 0, 0.55)",
      listening: false,
      visible: false,
    };
    dimMaskTop = new Konva.Rect({ ...dimAttrs, x: 0, y: 0, width: stageWidth, height: 0 });
    dimMaskBottom = new Konva.Rect({ ...dimAttrs, x: 0, y: 0, width: stageWidth, height: 0 });
    dimMaskLeft = new Konva.Rect({ ...dimAttrs, x: 0, y: 0, width: 0, height: stageHeight });
    dimMaskRight = new Konva.Rect({ ...dimAttrs, x: 0, y: 0, width: 0, height: stageHeight });
    overlayLayer.add(dimMaskTop);
    overlayLayer.add(dimMaskBottom);
    overlayLayer.add(dimMaskLeft);
    overlayLayer.add(dimMaskRight);

    const crosshairAttrs = {
      stroke: "rgba(255, 255, 255, 0.6)",
      strokeWidth: 1,
      listening: false,
      visible: false,
      dash: [4, 4],
    };
    crosshairH = new Konva.Line({ ...crosshairAttrs, points: [0, 0, 0, 0] });
    crosshairV = new Konva.Line({ ...crosshairAttrs, points: [0, 0, 0, 0] });
    overlayLayer.add(crosshairH);
    overlayLayer.add(crosshairV);

    cropSelectionRect = new Konva.Rect({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      stroke: "#4f46e5",
      strokeWidth: 2,
      dash: [6, 4],
      fill: "rgba(79, 70, 229, 0.15)",
      visible: false,
    });
    cropSelectionBorder = new Konva.Rect({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      stroke: "white",
      strokeWidth: 1,
      listening: false,
      visible: false,
    });
    overlayLayer.add(cropSelectionRect);
    overlayLayer.add(cropSelectionBorder);

    cropHandles = Array.from({ length: 8 }, () => {
      const handle = new Konva.Rect({
        x: 0,
        y: 0,
        width: CROP_HANDLE_SIZE,
        height: CROP_HANDLE_SIZE,
        fill: "white",
        stroke: "#4f46e5",
        strokeWidth: 1.5,
        visible: false,
        listening: true,
      });
      overlayLayer.add(handle);
      return handle;
    });

    marqueeRect = new Konva.Rect({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      stroke: "#4f46e5",
      strokeWidth: 1,
      dash: [3, 3],
      fill: "rgba(79, 70, 229, 0.08)",
      visible: false,
      listening: false,
    });
    annotationSelectionLayer.add(marqueeRect);

    stage.on("mousedown", (event) => {
      const pos = stage!.getPointerPosition();
      if (!pos) return;
      // Crop handles are real Konva nodes, so `event.target` is the handle
      // rather than the image. Test them before the Select-tool empty-canvas
      // branch, which otherwise returns early and makes all eight handles
      // visible but impossible to drag.
      const existing = cropSelectionGeometry();
      if (existing) {
        const hit = cropHandleHit(pos, existing);
        if (hit) {
          activeCropHandle = hit;
          dragging = true;
          startPoint = pos;
          return;
        }
      }
      // Drawing tools intercept the pointer once a crop exists.
      if (isDrawingTool()) {
        startDraft(pos);
        return;
      }
      // Select tool: handle annotation handles, body-drag, marquee.
      if (isSelectTool()) {
        // Handle drag first.
        const handleHit = annotationHandleHitTest(pos);
        if (handleHit) {
          const crop = cropCssRect();
          if (!crop) return;
          const local = pointerToCropLocal(pos, crop);
          if (!local) return;
          annotationStore.beginTransform(handleHit.id, handleHit.handle, local);
          annotationDragging = true;
          return;
        }
        // Click on an annotation: select it (shift-click toggles).
        const hit = annotationHitTest(pos);
        if (hit) {
          // If the click is on a selected annotation, default to a
          // translate gesture so the user can drag the selection.
          if (annotationStore.isSelected(hit.id)) {
            const crop = cropCssRect();
            if (crop) {
              const local = pointerToCropLocal(pos, crop);
              if (local) {
                annotationStore.beginTranslateSelection(local);
                annotationDragging = true;
              }
            }
            return;
          }
          if (event.evt.shiftKey) {
            annotationStore.selectAdd(hit.id);
          } else {
            annotationStore.selectOnly(hit.id);
          }
          rerenderSelection();
          return;
        }
        // Empty-canvas click: clear the selection (or, if shift is
        // held, begin a marquee that adds to the current selection).
        if (event.evt.shiftKey) {
          const crop = cropCssRect();
          const local = crop ? pointerToCropLocal(pos, crop) : null;
          if (!local) return;
          marqueeStart = local;
          marqueeRect!.position(local);
          marqueeRect!.size({ width: 0, height: 0 });
          marqueeRect!.visible(true);
          marqueeDragging = true;
        } else {
          annotationStore.clearSelection();
          rerenderSelection();
          // Fall through to the crop-region drag if the crop is
          // missing (the user might still be drawing the crop).
          if (event.target !== imageNode) return;
        }
      }
      if (event.target !== imageNode) return;
      startPoint = pos;
      cropSelectionRect!.position(pos);
      cropSelectionRect!.size({ width: 0, height: 0 });
      cropSelectionRect!.visible(true);
      dragging = true;
      activeCropHandle = null;
      redrawCropOverlay({ x: pos.x, y: pos.y, width: 0, height: 0 });
    });

    stage.on("mousemove", () => {
      const pos = stage!.getPointerPosition();
      if (!pos) return;
      pointerPos = pos;
      updateCrosshair();
      overlayLayer?.draw();
      if (drawingDraft) {
        continueDraft(pos);
        return;
      }
      if (annotationDragging) {
        const crop = cropCssRect();
        if (!crop) return;
        const local = pointerToCropLocal(pos, crop);
        if (!local) return;
        if (annotationStore.transform?.kind === "transform") {
          annotationStore.updateTransform(local);
        } else if (annotationStore.transform?.kind === "translate") {
          annotationStore.updateTranslateSelection(local);
        }
        rerenderAnnotations();
        rerenderSelection();
        return;
      }
      if (marqueeDragging && marqueeStart && marqueeRect) {
        const crop = cropCssRect();
        const local = crop ? pointerToCropLocal(pos, crop) : null;
        if (!local) return;
        const x = Math.min(marqueeStart.x, local.x);
        const y = Math.min(marqueeStart.y, local.y);
        const width = Math.abs(local.x - marqueeStart.x);
        const height = Math.abs(local.y - marqueeStart.y);
        marqueeRect.position({ x, y });
        marqueeRect.size({ width, height });
        annotationSelectionLayer?.draw();
        return;
      }
      if (!dragging || !startPoint) return;
      if (activeCropHandle) {
        const existing = cropSelectionGeometry();
        if (!existing) return;
        const next = applyCropHandleDrag(existing, activeCropHandle, pos);
        redrawCropOverlay(next);
      } else {
        const x = Math.min(startPoint.x, pos.x);
        const y = Math.min(startPoint.y, pos.y);
        const width = Math.abs(pos.x - startPoint.x);
        const height = Math.abs(pos.y - startPoint.y);
        redrawCropOverlay({ x, y, width, height });
      }
    });

    stage.on("mouseup", () => {
      if (drawingDraft) {
        endDraft();
        return;
      }
      if (annotationDragging) {
        annotationDragging = false;
        if (annotationStore.transform?.kind === "transform") {
          annotationStore.endTransform();
        } else if (annotationStore.transform?.kind === "translate") {
          annotationStore.endTranslateSelection();
        }
        rerenderAnnotations();
        rerenderSelection();
        return;
      }
      if (marqueeDragging && marqueeStart && marqueeRect) {
        const rect = {
          x: marqueeRect.x(),
          y: marqueeRect.y(),
          width: marqueeRect.width(),
          height: marqueeRect.height(),
        };
        marqueeDragging = false;
        marqueeStart = null;
        marqueeRect.visible(false);
        if (rect.width >= MIN_SELECTION_DIM && rect.height >= MIN_SELECTION_DIM) {
          const physicalRect = {
            origin: { x: clampPositive(rect.x), y: clampPositive(rect.y) },
            size: {
              width: clampPositive(rect.width),
              height: clampPositive(rect.height),
            },
          };
          annotationStore.selectMarquee(physicalRect, "add");
          rerenderSelection();
        } else {
          annotationSelectionLayer?.draw();
        }
        return;
      }
      if (!dragging) return;
      dragging = false;
      activeCropHandle = null;
      const rect = cropSelectionGeometry();
      if (!rect || rect.width < MIN_SELECTION_DIM || rect.height < MIN_SELECTION_DIM) {
        redrawCropOverlay(null);
        emitPhysicalSelection(null);
        return;
      }
      redrawCropOverlay(rect);
      emitPhysicalSelection(rect);
    });

    stage.on("mouseleave", () => {
      pointerPos = null;
      crosshairH?.visible(false);
      crosshairV?.visible(false);
      overlayLayer?.draw();
    });

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      stage?.destroy();
    };
  });

  // Refresh the annotation + selection chrome whenever the store
  // changes (after a draw, undo, batch style, etc.).
  $effect(() => {
    // Touch every reactive dependency so the effect re-runs when any
    // of them changes.
    void annotationStore.tool;
    void annotationStore.color;
    void annotationStore.stroke;
    void annotationStore.annotations;
    void annotationStore.draft;
    void annotationStore.selection;
    void lastSelection;
    const width = stageWidth;
    const height = stageHeight;
    if (stage) {
      stage.size({ width, height });
      imageNode?.size({ width, height });
      const crop = cropCssRect();
      if (crop) redrawCropOverlay(crop);
    }
    rerenderAnnotations();
    rerenderSelection();
  });
</script>

<div
  class="stage-container"
  bind:this={container}
  data-testid="konva-stage"
  data-has-selection={lastSelection ? "true" : "false"}
  data-active-tool={annotationStore.tool}
  data-draft={annotationStore.draft ? "true" : "false"}
  data-annotation-selection={annotationStore.selection.size}
  style:width="{stageWidth}px"
  style:height="{stageHeight}px"
></div>

{#if textEditing && annotationStore.draft && annotationStore.draft.geometry.kind === "text" && cropCssRect()}
  {@const crop = cropCssRect()!}
  {@const draftText = annotationStore.draft.geometry}
  {@const scaleX = stageWidth / bounds.size.width}
  {@const scaleY = stageHeight / bounds.size.height}
  {@const left = draftText.origin.x * scaleX + crop.x}
  {@const top = draftText.origin.y * scaleY + crop.y}
  {@const width = draftText.size.width * scaleX}
  {@const height = draftText.size.height * scaleY}
  <textarea
    class="text-editor"
    data-testid="text-editor"
    use:focusOnMount
    style:left="{left}px"
    style:top="{top}px"
    style:width="{Math.max(width, 80)}px"
    style:height="{Math.max(height, 24)}px"
    onkeydown={(event) => {
      if (event.key === "Enter" && !event.shiftKey && !isComposing) {
        event.preventDefault();
        const target = event.currentTarget as HTMLTextAreaElement;
        annotationStore.commitText(target.value);
        textEditing = false;
        rerenderAnnotations();
      } else if (event.key === "Escape" && !isComposing) {
        event.preventDefault();
        annotationStore.cancelDraft();
        textEditing = false;
        rerenderAnnotations();
      }
    }}
    oncompositionstart={() => {
      isComposing = true;
    }}
    oncompositionend={() => {
      isComposing = false;
    }}
  ></textarea>
{/if}

<style>
  .stage-container {
    position: relative;
    width: 100%;
    height: 100%;
  }
  .text-editor {
    position: absolute;
    z-index: 10;
    background: rgba(255, 255, 255, 0.95);
    color: #141414;
    border: 1px solid #4f46e5;
    border-radius: 2px;
    padding: 4px;
    font-family: system-ui, sans-serif;
    font-size: 14px;
    line-height: 1.2;
    resize: none;
    outline: none;
    box-sizing: border-box;
    pointer-events: auto;
  }
</style>
