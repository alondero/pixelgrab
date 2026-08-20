<!--
  PinWindow - borderless TopMost reference window for a single captured
  image. The window is drag-anywhere, cursor-centered zoom, Ctrl+wheel
  opacity, and exposes Copy / Save As / Reset / Close context actions.

  The component is data-driven: it never mutates `view` directly. Every
  gesture becomes a `PinCommand` round-trip through the IPC; the Rust
  registry is the single source of truth for the transform.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";

  import { PIN_LIMITS, type PinViewModel } from "./types";
  import { pinStore } from "./pinStore.svelte";

  let { view }: { view: PinViewModel } = $props();

  let dragState: { pointer_id: number; last_x: number; last_y: number } | null = null;
  let contextMenuOpen = $state(false);
  let contextMenuX = $state(0);
  let contextMenuY = $state(0);
  let containerEl: HTMLDivElement | null = null;

  // Computed physical-pixel size for the inner image. The window size
  // rounds to whole pixels; the inner image displays whatever the
  // browser interprets (it does not have to be pixel-perfect).
  let widthPx = $derived(view.transform.windowSize.width);
  let heightPx = $derived(view.transform.windowSize.height);
  let opacityPct = $derived(Math.round(view.transform.opacity * 100));
  let assetUrl = $derived(view.source.pngPath ? convertFileSrc(view.source.pngPath) : "");

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    dragState = { pointer_id: event.pointerId, last_x: event.clientX, last_y: event.clientY };
  }

  function onPointerMove(event: PointerEvent) {
    if (!dragState || dragState.pointer_id !== event.pointerId) return;
    const dx = event.clientX - dragState.last_x;
    const dy = event.clientY - dragState.last_y;
    dragState.last_x = event.clientX;
    dragState.last_y = event.clientY;
    pinStore.applyCommand(view.id, { kind: "drag", dx, dy });
  }

  function onPointerUp(event: PointerEvent) {
    if (!dragState || dragState.pointer_id !== event.pointerId) return;
    dragState = null;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
  }

  function onDoubleClick() {
    // The spec lists double-click alongside Escape and the visible close
    // control as a dismissal route; the Reset 100% action lives in the
    // context menu.
    pinStore.runAction(view.id, "close");
  }

  function onWheel(event: WheelEvent) {
    if (event.ctrlKey) {
      // Ctrl+wheel: opacity.
      const step = event.deltaY < 0 ? 0.05 : -0.05;
      const next = clamp(
        view.transform.opacity + step,
        PIN_LIMITS.minOpacity,
        PIN_LIMITS.maxOpacity,
      );
      pinStore.applyCommand(view.id, { kind: "setOpacity", opacity: next });
    } else {
      // Wheel: cursor-centered zoom.
      const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1;
      const rect = containerEl?.getBoundingClientRect();
      const cursorX = rect ? event.clientX - rect.left : 0;
      const cursorY = rect ? event.clientY - rect.top : 0;
      pinStore.applyCommand(view.id, {
        kind: "zoom",
        factor,
        cursorX,
        cursorY,
      });
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      pinStore.runAction(view.id, "close");
    }
  }

  function onContextMenu(event: MouseEvent) {
    event.preventDefault();
    contextMenuX = event.clientX;
    contextMenuY = event.clientY;
    contextMenuOpen = true;
  }

  function closeContextMenu() {
    contextMenuOpen = false;
  }

  async function pickAction(action: "copy" | "save_as" | "reset" | "close") {
    closeContextMenu();
    await pinStore.runAction(view.id, action);
  }

  onMount(() => {
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("click", closeContextMenu);
  });

  onDestroy(() => {
    window.removeEventListener("keydown", onKeyDown);
    window.removeEventListener("click", closeContextMenu);
  });

  function clamp(value: number, min: number, max: number): number {
    if (!Number.isFinite(value)) return min;
    return Math.min(max, Math.max(min, value));
  }
</script>

<svelte:window onwheel={onWheel} />

<div
  bind:this={containerEl}
  class="pin-window"
  style="
    width: {widthPx}px;
    height: {heightPx}px;
    transform: translate({view.transform.position.x}px, {view.transform.position.y}px);
    opacity: {view.transform.opacity};
  "
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  ondblclick={onDoubleClick}
  oncontextmenu={onContextMenu}
  role="img"
  aria-label="Pinned capture; zoom {Math.round(view.transform.zoom * 100)}%; opacity {opacityPct}%"
>
  <img class="pin-image" src={assetUrl} alt="" draggable="false" />
  <button
    type="button"
    class="pin-close"
    aria-label="Close pin"
    onclick={(event) => {
      event.stopPropagation();
      pinStore.runAction(view.id, "close");
    }}
  >
    ×
  </button>
  {#if contextMenuOpen}
    <div
      class="pin-context-menu"
      style="left: {contextMenuX}px; top: {contextMenuY}px;"
      role="menu"
    >
      <button type="button" role="menuitem" onclick={() => pickAction("copy")}>Copy</button>
      <button type="button" role="menuitem" onclick={() => pickAction("save_as")}>Save As…</button>
      <button type="button" role="menuitem" onclick={() => pickAction("reset")}>Reset 100%</button>
      <button type="button" role="menuitem" onclick={() => pickAction("close")}>Close</button>
    </div>
  {/if}
</div>

<style>
  .pin-window {
    position: absolute;
    top: 0;
    left: 0;
    border: 1px solid rgba(0, 0, 0, 0.4);
    background: rgba(0, 0, 0, 0.02);
    box-shadow: 0 6px 12px rgba(0, 0, 0, 0.25);
    overflow: hidden;
    user-select: none;
    touch-action: none;
  }
  .pin-window:focus {
    outline: 2px solid #2b8cff;
    outline-offset: 2px;
  }
  .pin-image {
    width: 100%;
    height: 100%;
    display: block;
    pointer-events: none;
  }
  .pin-close {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 22px;
    height: 22px;
    border: none;
    background: rgba(0, 0, 0, 0.55);
    color: white;
    border-radius: 11px;
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
  }
  .pin-close:hover {
    background: rgba(0, 0, 0, 0.75);
  }
  .pin-context-menu {
    position: fixed;
    background: rgba(28, 28, 28, 0.95);
    color: white;
    border-radius: 4px;
    padding: 4px 0;
    min-width: 140px;
    z-index: 1000;
  }
  .pin-context-menu button {
    display: block;
    width: 100%;
    background: transparent;
    color: inherit;
    border: none;
    text-align: left;
    padding: 6px 12px;
    cursor: pointer;
    font: inherit;
  }
  .pin-context-menu button:hover {
    background: rgba(255, 255, 255, 0.1);
  }
</style>
