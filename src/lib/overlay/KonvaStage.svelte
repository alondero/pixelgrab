<script lang="ts">
  import { onMount } from "svelte";
  import Konva from "konva";
  import type { PhysicalBounds } from "$lib/ipc/types";

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
  let dragging = $state(false);
  let startPoint: { x: number; y: number } | null = null;
  let pointerPos = $state<{ x: number; y: number } | null>(null);
  let activeHandle: HandlePosition | null = null;
  let lastSelection = $state<PhysicalBounds | null>(null);

  type HandlePosition = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

  const HANDLE_SIZE = 10;

  function clampPositive(value: number) {
    return Math.max(0, Math.round(value));
  }

  function emitPhysicalSelection(
    rect: {
      x: number;
      y: number;
      width: number;
      height: number;
    } | null,
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

  function positionHandles(
    rect: {
      x: number;
      y: number;
      width: number;
      height: number;
    } | null,
  ) {
    if (handles.length !== 8 || !rect) {
      for (const handle of handles) handle.visible(false);
      return;
    }
    const positions: Array<{ x: number; y: number }> = [
      { x: rect.x - HANDLE_SIZE / 2, y: rect.y - HANDLE_SIZE / 2 }, // nw
      {
        x: rect.x + rect.width / 2 - HANDLE_SIZE / 2,
        y: rect.y - HANDLE_SIZE / 2,
      }, // n
      { x: rect.x + rect.width - HANDLE_SIZE / 2, y: rect.y - HANDLE_SIZE / 2 }, // ne
      {
        x: rect.x + rect.width - HANDLE_SIZE / 2,
        y: rect.y + rect.height / 2 - HANDLE_SIZE / 2,
      }, // e
      {
        x: rect.x + rect.width - HANDLE_SIZE / 2,
        y: rect.y + rect.height - HANDLE_SIZE / 2,
      }, // se
      {
        x: rect.x + rect.width / 2 - HANDLE_SIZE / 2,
        y: rect.y + rect.height - HANDLE_SIZE / 2,
      }, // s
      { x: rect.x - HANDLE_SIZE / 2, y: rect.y + rect.height - HANDLE_SIZE / 2 }, // sw
      {
        x: rect.x - HANDLE_SIZE / 2,
        y: rect.y + rect.height / 2 - HANDLE_SIZE / 2,
      }, // w
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
    rect: {
      x: number;
      y: number;
      width: number;
      height: number;
    },
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
    handle: handlePosition,
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

  // Local alias for clarity in the function signature above.
  type handlePosition = HandlePosition;

  function selectionGeometry(): {
    x: number;
    y: number;
    width: number;
    height: number;
  } | null {
    if (!selectionRect) return null;
    return {
      x: selectionRect.x(),
      y: selectionRect.y(),
      width: selectionRect.width(),
      height: selectionRect.height(),
    };
  }

  function redrawOverlay(
    rect: {
      x: number;
      y: number;
      width: number;
      height: number;
    } | null,
  ) {
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

  function handleKey(event: KeyboardEvent) {
    // The overlay listens for Escape (staged cancel) and Ctrl+C / Cmd+C
    // (commit). The handlers are bound on `window` because the overlay
    // window is borderless and does not receive keyboard focus by default.
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel?.();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c") {
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
    stage.add(imageLayer);
    stage.add(overlayLayer);

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
      overlayLayer.draw();
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
      overlayLayer.draw();
    });

    window.addEventListener("keydown", handleKey);
    return () => {
      window.removeEventListener("keydown", handleKey);
      stage?.destroy();
    };
  });
</script>

<div
  class="stage-container"
  bind:this={container}
  data-testid="konva-stage"
  data-has-selection={lastSelection ? "true" : "false"}
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
