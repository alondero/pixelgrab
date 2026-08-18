<script lang="ts">
  import { onMount } from "svelte";
  import Konva from "konva";
  import type { PhysicalBounds } from "$lib/ipc/types";

  interface Props {
    assetUrl: string;
    bounds: PhysicalBounds;
    onSelectionChange: (bounds: PhysicalBounds | null) => void;
  }

  let { assetUrl, bounds, onSelectionChange }: Props = $props();

  let container: HTMLDivElement;
  let stage: Konva.Stage | null = null;
  let imageNode: Konva.Image | null = null;
  let selectionRect: Konva.Rect | null = null;
  let dragging = $state(false);
  let startPoint: { x: number; y: number } | null = null;

  onMount(() => {
    stage = new Konva.Stage({
      container,
      width: container.clientWidth,
      height: container.clientHeight,
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
    overlayLayer.add(selectionRect);

    stage.on("mousedown", (event) => {
      if (event.target !== imageNode) return;
      const pos = stage!.getPointerPosition();
      if (!pos) return;
      startPoint = pos;
      selectionRect!.position(pos);
      selectionRect!.size({ width: 0, height: 0 });
      selectionRect!.visible(true);
      overlayLayer.draw();
      dragging = true;
    });

    stage.on("mousemove", () => {
      if (!dragging || !startPoint) return;
      const pos = stage!.getPointerPosition();
      if (!pos) return;
      const x = Math.min(startPoint.x, pos.x);
      const y = Math.min(startPoint.y, pos.y);
      const width = Math.abs(pos.x - startPoint.x);
      const height = Math.abs(pos.y - startPoint.y);
      selectionRect!.position({ x, y });
      selectionRect!.size({ width, height });
      overlayLayer.draw();
    });

    stage.on("mouseup", () => {
      if (!dragging) return;
      dragging = false;
      const x = selectionRect!.x();
      const y = selectionRect!.y();
      const width = selectionRect!.width();
      const height = selectionRect!.height();
      if (width < 4 || height < 4) {
        selectionRect!.visible(false);
        overlayLayer.draw();
        onSelectionChange(null);
        return;
      }
      // Convert stage coordinates back into physical pixel coordinates.
      const scaleX = bounds.size.width / stage!.width();
      const scaleY = bounds.size.height / stage!.height();
      onSelectionChange({
        origin: {
          x: bounds.origin.x + Math.round(x * scaleX),
          y: bounds.origin.y + Math.round(y * scaleY),
        },
        size: {
          width: Math.round(width * scaleX),
          height: Math.round(height * scaleY),
        },
      });
    });

    return () => stage?.destroy();
  });
</script>

<div class="stage-container" bind:this={container} data-testid="konva-stage"></div>

<style>
  .stage-container {
    flex: 1;
    width: 100%;
    height: 100%;
  }
</style>
