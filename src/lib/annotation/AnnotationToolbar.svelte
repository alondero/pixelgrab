<script lang="ts">
  // Floating annotation toolbar. Renders only when an annotation tool
  // is active; lets the user pick tool, color, and stroke width.
  // Keyboard shortcuts are bound by the parent (KonvaStage) so the
  // toolbar can stay focused on rendering.
  //
  // Tracer 06: when a non-empty selection is active, the toolbar
  // reflects the selection's colour + stroke (with a "mixed" indicator
  // when the set spans multiple values). Clicking a swatch / stroke
  // calls `applyColorToSelection` / `applyStrokeToSelection`, which
  // updates every selected annotation in a single history entry.
  // With an empty selection the toolbar behaves as before (next-draw
  // style).

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
    { id: "text", label: "Text", key: "T", glyph: "T" },
    { id: "blur", label: "Blur", key: "B", glyph: "▒" },
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

  // Resolved toolbar state sourced from the selection. With an empty
  // selection we fall back to the next-draw colour / stroke so the
  // shortcut behaves the same with or without a selection.
  const selectionColor = $derived(annotationStore.selectionColor());
  const selectionStroke = $derived(annotationStore.selectionStroke());
  /// The colour the toolbar should highlight as "active". `null` =
  /// no selection (next-draw fallback); `"mixed"` = heterogeneous
  /// selection (every swatch is shown lit on hover only).
  const activeColor = $derived(selectionColor === null ? annotationStore.color : selectionColor);
  const activeStroke = $derived(
    selectionStroke === null ? annotationStore.stroke : selectionStroke,
  );

  function selectTool(tool: AnnotationTool) {
    annotationStore.setTool(tool);
  }
  function selectColor(color: AnnotationColor) {
    // Routes through the batch setter so a non-empty selection
    // updates every selected annotation in a single history entry.
    annotationStore.applyColorToSelection(color);
  }
  function selectStroke(stroke: AnnotationStroke) {
    annotationStore.applyStrokeToSelection(stroke);
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
      <span
        class="state-pill"
        data-testid="color-state"
        data-state={selectionColor === "mixed"
          ? "mixed"
          : selectionColor === null
            ? "none"
            : "selected"}
      >
        {selectionColor === "mixed" ? "Mixed" : selectionColor === null ? "Next" : "Selected"}
      </span>
      {#each COLORS as color (color.id)}
        <button
          type="button"
          class="swatch"
          class:active={activeColor === color.id}
          class:indeterminate={selectionColor === "mixed"}
          aria-pressed={activeColor === color.id}
          aria-label="{color.label} color"
          data-testid="color-{color.id}"
          data-active={selectionColor === "mixed"
            ? "mixed"
            : activeColor === color.id
              ? "active"
              : "inactive"}
          style="--swatch: {color.swatch}"
          onclick={() => selectColor(color.id)}
        ></button>
      {/each}
    </div>
    <div class="divider" aria-hidden="true"></div>
    <div class="group" data-testid="stroke-group" aria-label="Stroke width">
      <span
        class="state-pill"
        data-testid="stroke-state"
        data-state={selectionStroke === "mixed"
          ? "mixed"
          : selectionStroke === null
            ? "none"
            : "selected"}
      >
        {selectionStroke === "mixed" ? "Mixed" : selectionStroke === null ? "Next" : "Selected"}
      </span>
      {#each STROKES as stroke (stroke.id)}
        <button
          type="button"
          class="stroke"
          class:active={activeStroke === stroke.id}
          class:indeterminate={selectionStroke === "mixed"}
          aria-pressed={activeStroke === stroke.id}
          aria-label="{stroke.label} ({stroke.px} pixels)"
          data-testid="stroke-{stroke.id}"
          data-active={selectionStroke === "mixed"
            ? "mixed"
            : activeStroke === stroke.id
              ? "active"
              : "inactive"}
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
  /* Mixed-selection swatch: dim the active highlight so the user
     can see the next click will resolve the heterogeneity. */
  .swatch.indeterminate {
    border-style: dashed;
    border-color: rgba(79, 70, 229, 0.5);
  }
  .stroke.indeterminate {
    border-style: dashed;
    border-color: rgba(79, 70, 229, 0.5);
  }
  .state-pill {
    font-size: 0.65rem;
    padding: 0.1rem 0.4rem;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.18);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.85;
    margin-right: 0.25rem;
  }
  .state-pill[data-state="mixed"] {
    background: rgba(246, 227, 59, 0.18);
    border-color: rgba(246, 227, 59, 0.5);
    color: #f6e33b;
  }
  .state-pill[data-state="selected"] {
    background: rgba(79, 70, 229, 0.18);
    border-color: rgba(79, 70, 229, 0.5);
    color: #c7d2fe;
  }
  .state-pill[data-state="none"] {
    opacity: 0.5;
  }
  .stroke-bar {
    display: inline-block;
    width: 24px;
    height: var(--bar-height);
    background: white;
    border-radius: 2px;
  }
</style>
