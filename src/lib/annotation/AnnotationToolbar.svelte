<script lang="ts">
  // Floating annotation toolbar. Renders only when an annotation tool
  // is active; lets the user pick tool, color, and stroke width.
  // Keyboard shortcuts are bound by the parent (KonvaStage) so the
  // toolbar can stay focused on rendering.

  import { annotationStore } from "./store.svelte";
  import type { AnnotationColor, AnnotationStroke, AnnotationTool } from "$lib/ipc/types";

  interface Props {
    visible?: boolean;
  }
  let { visible = true }: Props = $props();

  type ToolDef = { id: AnnotationTool; label: string; key: string; glyph: string };
  const TOOLS: ToolDef[] = [
    { id: "select", label: "Select", key: "V", glyph: "↖" },
    { id: "arrow", label: "Arrow", key: "A", glyph: "↗" },
    { id: "rectangle", label: "Rectangle", key: "R", glyph: "▭" },
    { id: "numbered_badge", label: "Badge", key: "N", glyph: "1" },
  ];

  type ColorDef = { id: AnnotationColor; label: string; swatch: string };
  const COLORS: ColorDef[] = [
    { id: "red", label: "Red", swatch: "#e53b3b" },
    { id: "green", label: "Green", swatch: "#3be55c" },
    { id: "blue", label: "Blue", swatch: "#3b82e5" },
    { id: "yellow", label: "Yellow", swatch: "#f6e33b" },
    { id: "white", label: "White", swatch: "#ffffff" },
  ];

  type StrokeDef = { id: AnnotationStroke; label: string; px: number };
  const STROKES: StrokeDef[] = [
    { id: "thin", label: "Thin", px: 2 },
    { id: "medium", label: "Medium", px: 4 },
    { id: "thick", label: "Thick", px: 8 },
  ];

  function selectTool(tool: AnnotationTool) {
    annotationStore.setTool(tool);
  }
  function selectColor(color: AnnotationColor) {
    annotationStore.setColor(color);
  }
  function selectStroke(stroke: AnnotationStroke) {
    annotationStore.setStroke(stroke);
  }
  function runUndo() {
    annotationStore.undo();
  }
  function runRedo() {
    annotationStore.redo();
  }
</script>

{#if visible}
  <div
    class="toolbar"
    data-testid="annotation-toolbar"
    role="toolbar"
    aria-label="Annotation tools"
  >
    <div class="group" data-testid="tool-group" aria-label="Tools">
      {#each TOOLS as tool (tool.id)}
        <button
          type="button"
          class="tool"
          class:active={annotationStore.tool === tool.id}
          aria-pressed={annotationStore.tool === tool.id}
          aria-label="{tool.label} (shortcut {tool.key})"
          data-testid="tool-{tool.id}"
          onclick={() => selectTool(tool.id)}
        >
          <span class="glyph" aria-hidden="true">{tool.glyph}</span>
          <span class="kbd" aria-hidden="true">{tool.key}</span>
        </button>
      {/each}
    </div>
    <div class="divider" aria-hidden="true"></div>
    <div class="group" data-testid="color-group" aria-label="Color">
      {#each COLORS as color (color.id)}
        <button
          type="button"
          class="swatch"
          class:active={annotationStore.color === color.id}
          aria-pressed={annotationStore.color === color.id}
          aria-label="{color.label} color"
          data-testid="color-{color.id}"
          style="--swatch: {color.swatch}"
          onclick={() => selectColor(color.id)}
        ></button>
      {/each}
    </div>
    <div class="divider" aria-hidden="true"></div>
    <div class="group" data-testid="stroke-group" aria-label="Stroke width">
      {#each STROKES as stroke (stroke.id)}
        <button
          type="button"
          class="stroke"
          class:active={annotationStore.stroke === stroke.id}
          aria-pressed={annotationStore.stroke === stroke.id}
          aria-label="{stroke.label} ({stroke.px} pixels)"
          data-testid="stroke-{stroke.id}"
          onclick={() => selectStroke(stroke.id)}
        >
          <span class="stroke-bar" style="--bar-height: {stroke.px}px"></span>
          <span class="kbd">{stroke.label}</span>
        </button>
      {/each}
    </div>
    <div class="divider" aria-hidden="true"></div>
    <div class="group" data-testid="history-group" aria-label="History">
      <button
        type="button"
        class="action"
        aria-label="Undo (Ctrl+Z)"
        data-testid="undo"
        disabled={!annotationStore.canUndo}
        onclick={runUndo}
      >
        ↶
      </button>
      <button
        type="button"
        class="action"
        aria-label="Redo (Ctrl+Shift+Z)"
        data-testid="redo"
        disabled={!annotationStore.canRedo}
        onclick={runRedo}
      >
        ↷
      </button>
    </div>
  </div>
{/if}

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    background: rgba(20, 20, 28, 0.92);
    color: white;
    border-radius: 8px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
    font-family: system-ui, sans-serif;
    font-size: 0.85rem;
    user-select: none;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
  .divider {
    width: 1px;
    height: 24px;
    background: rgba(255, 255, 255, 0.15);
  }
  .tool,
  .stroke,
  .action {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.35rem 0.6rem;
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 4px;
    color: inherit;
    cursor: pointer;
  }
  .tool.active,
  .stroke.active {
    background: #4f46e5;
    border-color: #4f46e5;
  }
  .action:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .glyph {
    font-size: 1rem;
    line-height: 1;
  }
  .kbd {
    font-size: 0.7rem;
    opacity: 0.7;
  }
  .swatch {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid transparent;
    background: var(--swatch);
    cursor: pointer;
    padding: 0;
  }
  .swatch.active {
    border-color: #4f46e5;
    box-shadow: 0 0 0 2px rgba(79, 70, 229, 0.4);
  }
  .stroke-bar {
    display: inline-block;
    width: 24px;
    height: var(--bar-height);
    background: white;
    border-radius: 2px;
  }
</style>
