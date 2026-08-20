<script lang="ts">
  // Konva-driven overlay stage. Hosts two layered UI surfaces:
  //   1. The frozen-frame image plus the dim mask, crosshair, and
  //      region-selection rectangle from tracer-02.
  //   2. The annotation layer (tracer-04): renders every annotation
  //      plus the in-flight draft, captures pointer events for the
  //      drawing tools, and binds the toolbar's keyboard shortcuts.
  //
  // The two surfaces share a single pointer pipeline. When the active
  // tool is `select`, the region-selection rectangle owns the pointer.
  // When the tool is arrow/rectangle/numbered_badge, the annotation
  // pipeline owns the pointer. The crop must already be committed
  // before drawing tools become active (a draft outside the crop has
  // no meaning for the flattened output).

  import { onMount } from "svelte";
  import Konva from "konva";
  import type { Annotation, PhysicalBounds, PhysicalPoint } from "$lib/ipc/types";
  import { annotationStore } from "$lib/annotation/store.svelte";
  import type { AnnotationColor, AnnotationStroke } from "$lib/ipc/types";

  interface Props {
    assetUrl: string;
    bounds: PhysicalBounds;
    stageWidth: number;
    stageHeight: number;
    onSelectionChange: (bounds: PhysicalBounds | null) => void;
    onCommit?: () => void;
    onCancel?: () => void;
  }

  let { assetUrl, bounds, stageWidth, stageHeight, onSelectionChange, onCommit, onCancel }: Props =
    $props();

  let container: HTMLDivElement;
  let stage: Konva.Stage | null = null;
  let imageNode: Konva.Image | null = null;
  let dimMaskTop: Konva.Rect | null = null;
  let dimMaskBottom: Konva.Rect | null = null;
  let dimMaskLeft: Konva.Rect | null = null;
  let dimMaskRight: Konva.Rect | null = null;
  let crosshairH: Konva.Line | null = null;
  let crosshairV: Konva.Line | null = null;
  let selectionRect: Konva.Rect | null = null;
  let selectionBorder: Konva.Rect | null = null;
  let handles: Konva.Rect[] = [];
  // Annotation layer nodes.
  let annotationLayer: Konva.Layer | null = null;
  let annotationNodes = new Map<number, Konva.Group>();
  let draftNode: Konva.Group | null = null;
  // Region selection state (tracer-02).
  let dragging = $state(false);
  let startPoint: { x: number; y: number } | null = null;
  let pointerPos = $state<{ x: number; y: number } | null>(null);
  let activeHandle: HandlePosition | null = null;
  let lastSelection = $state<PhysicalBounds | null>(null);
  // Region-lock flag: once a selection is committed, drawing tools
  // take over the pointer until the user cancels or commits.
  let drawingDraft = $state(false);
  let draftStart: PhysicalPoint | null = null;

  type HandlePosition = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

  const HANDLE_SIZE = 10;

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
    // The overlay renders the capture 1:1 (stageWidth matches
    // bounds.size.width at 100% browser zoom); the scale is the
    // inverse of that ratio so a future tracer can introduce
    // per-monitor DPI without changing this code.
    const scaleX = bounds.size.width / stageWidth;
    const scaleY = bounds.size.height / stageHeight;
    return {
      x: clampPositive((pos.x - crop.x) * scaleX),
      y: clampPositive((pos.y - crop.y) * scaleY),
    };
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

  function positionHandles(rect: { x: number; y: number; width: number; height: number } | null) {
    if (handles.length !== 8 || !rect) {
      for (const handle of handles) handle.visible(false);
      return;
    }
    const positions: Array<{ x: number; y: number }> = [
      { x: rect.x - HANDLE_SIZE / 2, y: rect.y - HANDLE_SIZE / 2 },
      { x: rect.x + rect.width / 2 - HANDLE_SIZE / 2, y: rect.y - HANDLE_SIZE / 2 },
      { x: rect.x + rect.width - HANDLE_SIZE / 2, y: rect.y - HANDLE_SIZE / 2 },
      { x: rect.x + rect.width - HANDLE_SIZE / 2, y: rect.y + rect.height / 2 - HANDLE_SIZE / 2 },
      { x: rect.x + rect.width - HANDLE_SIZE / 2, y: rect.y + rect.height - HANDLE_SIZE / 2 },
      { x: rect.x + rect.width / 2 - HANDLE_SIZE / 2, y: rect.y + rect.height - HANDLE_SIZE / 2 },
      { x: rect.x - HANDLE_SIZE / 2, y: rect.y + rect.height - HANDLE_SIZE / 2 },
      { x: rect.x - HANDLE_SIZE / 2, y: rect.y + rect.height / 2 - HANDLE_SIZE / 2 },
    ];
    positions.forEach((pos, index) => {
      const handle = handles[index];
      handle.position(pos);
      handle.size({ width: HANDLE_SIZE, height: HANDLE_SIZE });
      handle.visible(true);
    });
  }

  function handleHit(
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
    const tolerance = HANDLE_SIZE;
    for (const [name, hx, hy] of positions) {
      if (Math.abs(pos.x - hx) <= tolerance && Math.abs(pos.y - hy) <= tolerance) {
        return name;
      }
    }
    return null;
  }

  function applyHandleDrag(
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

  function selectionGeometry(): { x: number; y: number; width: number; height: number } | null {
    if (!selectionRect) return null;
    return {
      x: selectionRect.x(),
      y: selectionRect.y(),
      width: selectionRect.width(),
      height: selectionRect.height(),
    };
  }

  function redrawOverlay(rect: { x: number; y: number; width: number; height: number } | null) {
    if (!selectionRect || !selectionBorder || !stage) return;
    if (!rect) {
      selectionRect.visible(false);
      selectionBorder.visible(false);
      updateDimMask(null);
      positionHandles(null);
      return;
    }
    selectionRect.position({ x: rect.x, y: rect.y });
    selectionRect.size({ width: rect.width, height: rect.height });
    selectionRect.visible(true);
    selectionBorder.position({ x: rect.x, y: rect.y });
    selectionBorder.size({ width: rect.width, height: rect.height });
    selectionBorder.visible(true);
    updateDimMask(rect);
    positionHandles(rect);
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
    if (annotation.geometry.kind === "arrow") {
      const group = new Konva.Group({ listening: false, opacity });
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
      return group;
    }
    if (annotation.geometry.kind === "rectangle") {
      const { origin, size } = annotation.geometry;
      const group = new Konva.Group({ listening: false, opacity });
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
      return group;
    }
    // Numbered badge: a filled circle plus a centred digit drawn as a
    // Group so the digit text overlays the fill.
    const { center, radius } = annotation.geometry;
    const group = new Konva.Group({ listening: false, opacity });
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
          // Mirror the Rust rasterizer's luminance rule: dark text on
          // light fills, light text on dark fills. White badges must
          // not paint an invisible "1".
          fill: digitFillForColor(color),
          listening: false,
        }),
      );
    }
    return group;
  }

  /// Compute the badge digit colour that contrasts with the badge
  /// fill. Mirrors `digit_color_for_luminance` in the Rust rasterizer
  /// so the Konva preview and the flattened PNG agree. Takes the hex
  /// string form (already resolved from `COLOR_HEX`) so callers do
  /// not need to re-resolve the enum.
  function digitFillForColor(hex: string): string {
    const h = hex.replace("#", "");
    const r = parseInt(h.slice(0, 2), 16) / 255;
    const g = parseInt(h.slice(2, 4), 16) / 255;
    const b = parseInt(h.slice(4, 6), 16) / 255;
    const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    return lum < 0.5 ? "#ffffff" : "#141414";
  }

  function rerenderAnnotations() {
    if (!annotationLayer) return;
    annotationLayer.destroyChildren();
    annotationNodes = new Map();
    // Translate the layer so crop-local coordinates map onto stage CSS
    // coordinates: the crop's CSS origin is the layer offset.
    const crop = cropCssRect();
    if (crop) {
      annotationLayer.position({ x: crop.x, y: crop.y });
    } else {
      annotationLayer.position({ x: 0, y: 0 });
    }
    for (const annotation of annotationStore.annotations) {
      const node = annotationNode(annotation, false);
      annotationLayer.add(node);
      annotationNodes.set(annotation.id, node);
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

  // ---- Pointer / keyboard handlers ---------------------------------

  function isDrawingTool(): boolean {
    return (
      annotationStore.tool === "arrow" ||
      annotationStore.tool === "rectangle" ||
      annotationStore.tool === "numbered_badge"
    );
  }

  function startDraft(pos: { x: number; y: number }) {
    if (!isDrawingTool()) return;
    const crop = cropCssRect();
    if (!crop) return; // No crop yet — drawing is disabled.
    const local = pointerToCropLocal(pos, crop);
    if (!local) return;
    drawingDraft = true;
    draftStart = local;
    const kind =
      annotationStore.tool === "arrow"
        ? "arrow"
        : annotationStore.tool === "rectangle"
          ? "rectangle"
          : "numbered_badge";
    annotationStore.beginDraft(kind, local);
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
    // For badges we always commit (a single click is a valid badge).
    // For arrows and rectangles, only commit if the shape has area.
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
    }
    annotationStore.commitDraft();
    rerenderAnnotations();
  }

  function handleKey(event: KeyboardEvent) {
    // Annotation shortcuts: A/R/N/V switch tools; Ctrl+Z / Ctrl+Shift+Z
    // operate on history. Escape staged: first clears the draft, then
    // cancels the session via `onCancel`.
    const key = event.key.toLowerCase();
    if (event.ctrlKey || event.metaKey) {
      if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        annotationStore.undo();
        rerenderAnnotations();
        return;
      }
      if ((key === "z" && event.shiftKey) || key === "y") {
        event.preventDefault();
        annotationStore.redo();
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
      }
    }
    if (event.key === "Escape") {
      // First Escape: drop any in-flight draft. Second Escape: clear
      // the crop. Final Escape: cancel the session.
      if (annotationStore.draft) {
        event.preventDefault();
        annotationStore.cancelDraft();
        rerenderAnnotations();
        return;
      }
      event.preventDefault();
      onCancel?.();
      return;
    }
    if (((event.ctrlKey || event.metaKey) && key === "c") || event.key === "Enter") {
      if (lastSelection) {
        event.preventDefault();
        onCommit?.();
      }
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
    annotationLayer = new Konva.Layer({ listening: false });
    stage.add(imageLayer);
    stage.add(overlayLayer);
    stage.add(annotationLayer);

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
    img.src = assetUrl;

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

    selectionRect = new Konva.Rect({
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
    selectionBorder = new Konva.Rect({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      stroke: "white",
      strokeWidth: 1,
      listening: false,
      visible: false,
    });
    overlayLayer.add(selectionRect);
    overlayLayer.add(selectionBorder);

    handles = Array.from({ length: 8 }, () => {
      const handle = new Konva.Rect({
        x: 0,
        y: 0,
        width: HANDLE_SIZE,
        height: HANDLE_SIZE,
        fill: "white",
        stroke: "#4f46e5",
        strokeWidth: 1.5,
        visible: false,
        listening: true,
      });
      overlayLayer.add(handle);
      return handle;
    });

    stage.on("mousedown", (event) => {
      const pos = stage!.getPointerPosition();
      if (!pos) return;
      // Drawing tools intercept the pointer once a crop exists.
      if (isDrawingTool()) {
        startDraft(pos);
        return;
      }
      const existing = selectionGeometry();
      if (existing) {
        const hit = handleHit(pos, existing);
        if (hit) {
          activeHandle = hit;
          dragging = true;
          startPoint = pos;
          return;
        }
      }
      if (event.target !== imageNode) return;
      startPoint = pos;
      selectionRect!.position(pos);
      selectionRect!.size({ width: 0, height: 0 });
      selectionRect!.visible(true);
      dragging = true;
      activeHandle = null;
      redrawOverlay({ x: pos.x, y: pos.y, width: 0, height: 0 });
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
      if (!dragging || !startPoint) return;
      if (activeHandle && selectionRect) {
        const existing = selectionGeometry();
        if (!existing) return;
        const next = applyHandleDrag(existing, activeHandle, pos);
        redrawOverlay(next);
      } else {
        const x = Math.min(startPoint.x, pos.x);
        const y = Math.min(startPoint.y, pos.y);
        const width = Math.abs(pos.x - startPoint.x);
        const height = Math.abs(pos.y - startPoint.y);
        redrawOverlay({ x, y, width, height });
      }
    });

    stage.on("mouseup", () => {
      if (drawingDraft) {
        endDraft();
        return;
      }
      if (!dragging) return;
      dragging = false;
      activeHandle = null;
      const rect = selectionGeometry();
      if (!rect || rect.width < 4 || rect.height < 4) {
        redrawOverlay(null);
        emitPhysicalSelection(null);
        return;
      }
      redrawOverlay(rect);
      emitPhysicalSelection(rect);
    });

    stage.on("mouseleave", () => {
      pointerPos = null;
      crosshairH?.visible(false);
      crosshairV?.visible(false);
      overlayLayer?.draw();
    });

    window.addEventListener("keydown", handleKey);
    return () => {
      window.removeEventListener("keydown", handleKey);
      stage?.destroy();
    };
  });

  // Whenever the store annotations change (after an undo, a tool
  // change that resets state, etc.) re-render the layer.
  $effect(() => {
    // Touch every reactive dependency so the effect re-runs when any
    // of them changes.
    void annotationStore.tool;
    void annotationStore.color;
    void annotationStore.stroke;
    void annotationStore.annotations;
    void annotationStore.draft;
    void lastSelection;
    rerenderAnnotations();
  });
</script>

<div
  class="stage-container"
  bind:this={container}
  data-testid="konva-stage"
  data-has-selection={lastSelection ? "true" : "false"}
  data-active-tool={annotationStore.tool}
  data-draft={annotationStore.draft ? "true" : "false"}
  style:width="{stageWidth}px"
  style:height="{stageHeight}px"
></div>

<style>
  .stage-container {
    position: relative;
    width: 100%;
    height: 100%;
  }
</style>
