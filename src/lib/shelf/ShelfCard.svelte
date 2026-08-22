<script lang="ts">
  import type { ShelfQueueCard } from "$lib/ipc/types";
  import { formatRemaining, remainingMs } from "./queue.svelte";

  // One shelf card. Renders the thumbnail, the editable title, the
  // countdown, and the quick-action buttons. The parent component
  // (ShelfQueue) wires the `on*` callbacks to IPC commands.
  //
  // Issue #63: the card also exposes Pin (independent TopMost window),
  // Edit (reopen for non-destructive revision), and a native drag
  // gesture — a pointer press that moves past `DRAG_THRESHOLD_PX`
  // fires `onDrag` exactly once per gesture; the backend owns the OLE
  // drag loop from there.
  let {
    card,
    nowMs,
    onCopy = () => {},
    onSaveAs = () => {},
    onDismiss = () => {},
    onHover = () => {},
    onUnhover = () => {},
    onPin = () => {},
    onEdit = () => {},
    onDrag = () => {},
  }: {
    card: ShelfQueueCard;
    nowMs: number;
    onCopy?: (shelfId: string) => void;
    onSaveAs?: (shelfId: string) => void;
    onDismiss?: (shelfId: string) => void;
    onHover?: (shelfId: string) => void;
    onUnhover?: (shelfId: string) => void;
    onPin?: (shelfId: string) => void;
    onEdit?: (card: ShelfQueueCard) => void;
    onDrag?: (card: ShelfQueueCard) => void;
  } = $props();

  const DRAG_THRESHOLD_PX = 8;

  // Native drag gesture state. One gesture = one pointerdown → up
  // sequence; only a movement past the threshold fires the callback.
  let dragGestureActive = false;
  let dragFired = false;
  let dragStartX = 0;
  let dragStartY = 0;

  function onPointerDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    dragGestureActive = true;
    dragFired = false;
    dragStartX = event.clientX;
    dragStartY = event.clientY;
  }

  function onPointerMove(event: PointerEvent): void {
    if (!dragGestureActive || dragFired) return;
    const dx = event.clientX - dragStartX;
    const dy = event.clientY - dragStartY;
    if (Math.hypot(dx, dy) >= DRAG_THRESHOLD_PX) {
      dragFired = true;
      onDrag(card);
    }
  }

  function onPointerUp(): void {
    dragGestureActive = false;
  }

  type TauriGlobal = {
    core?: { convertFileSrc?: (p: string) => string };
  };
  function getTauri(): TauriGlobal | undefined {
    return (globalThis as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
  }
  function convertFileSrc(p: string): string {
    return getTauri()?.core?.convertFileSrc?.(p) ?? p;
  }

  let pngUrl = $derived(convertFileSrc(card.pngPath));
  let titleText = $derived(card.metadata.title?.trim() || "Untitled capture");
  let remaining = $derived(remainingMs(card.timer, nowMs));
  let countdownText = $derived(formatRemaining(remaining));
  let paused = $derived(card.timer.pausedAtElapsedMs !== undefined);
  let expired = $derived(remaining <= 0 && !paused);
</script>

<div
  class="card"
  class:paused
  class:expired
  data-testid="shelf-card"
  data-shelf-id={card.shelfId}
  aria-label="Shelf card: {titleText}"
  role="region"
  onmouseenter={() => onHover(card.shelfId)}
  onmouseleave={() => onUnhover(card.shelfId)}
  onfocusin={() => onHover(card.shelfId)}
  onfocusout={() => onUnhover(card.shelfId)}
>
  <div
    class="drag-surface"
    data-testid="shelf-drag-surface"
    role="presentation"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
  >
    <div class="thumbnail" data-testid="shelf-thumbnail">
      <img src={pngUrl} alt={titleText} draggable="false" />
    </div>
    <div class="meta">
      <div class="title" data-testid="shelf-title">{titleText}</div>
      <div class="row">
        <span class="size" data-testid="shelf-size">
          {Math.round(card.sizeBytes / 1024)} KB
        </span>
        <span
          class="countdown"
          class:paused
          class:expired
          data-testid="shelf-countdown"
          aria-label={paused ? "Card timer paused" : "Card timer remaining"}
        >
          {countdownText}
        </span>
      </div>
    </div>
  </div>
  <div class="actions">
    <button
      type="button"
      class="action"
      data-testid="shelf-copy"
      aria-label="Copy card to clipboard"
      onclick={() => onCopy(card.shelfId)}
    >
      Copy
    </button>
    <button
      type="button"
      class="action"
      data-testid="shelf-save-as"
      aria-label="Save card as PNG"
      onclick={() => onSaveAs(card.shelfId)}
    >
      Save
    </button>
    <button
      type="button"
      class="action"
      data-testid="shelf-pin"
      aria-label="Pin card as reference window"
      onclick={() => onPin(card.shelfId)}
    >
      Pin
    </button>
    <button
      type="button"
      class="action"
      data-testid="shelf-edit"
      aria-label="Reopen card for editing"
      onclick={() => onEdit(card)}
    >
      Edit
    </button>
    <button
      type="button"
      class="dismiss"
      data-testid="shelf-dismiss"
      aria-label="Dismiss shelf card"
      onclick={() => onDismiss(card.shelfId)}
    >
      ×
    </button>
  </div>
</div>

<style>
  .card {
    width: 200px;
    height: 150px;
    background: rgba(28, 28, 32, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 8px;
    display: grid;
    grid-template-columns: 1fr;
    grid-template-rows: 1fr auto;
    grid-template-areas:
      "surface"
      "actions";
    gap: 6px;
    padding: 6px 8px;
    color: #fff;
    font-family: system-ui, sans-serif;
    box-sizing: border-box;
    overflow: hidden;
    transition: opacity 200ms ease;
  }
  .drag-surface {
    grid-area: surface;
    display: grid;
    grid-template-columns: 56px 1fr;
    gap: 6px;
    min-height: 0;
    cursor: grab;
  }
  .drag-surface:active {
    cursor: grabbing;
  }
  .card.paused {
    border-color: rgba(78, 161, 255, 0.6);
  }
  .card.expired {
    opacity: 0.35;
  }
  .thumbnail {
    grid-area: thumbnail;
    width: 56px;
    height: 56px;
    background: #000;
    border-radius: 4px;
    overflow: hidden;
    align-self: start;
  }
  .thumbnail img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .meta {
    grid-area: meta;
    min-width: 0;
  }
  .title {
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 4px;
    margin-top: 2px;
  }
  .size {
    font-size: 10px;
    opacity: 0.7;
  }
  .countdown {
    font-size: 10px;
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
  }
  .countdown.paused {
    color: #4ea1ff;
    opacity: 1;
  }
  .countdown.expired {
    color: #ff7a7a;
    opacity: 1;
  }
  .actions {
    grid-area: actions;
    display: flex;
    gap: 4px;
    justify-content: flex-end;
  }
  .action,
  .dismiss {
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.24);
    color: #fff;
    border-radius: 4px;
    cursor: pointer;
    font-family: system-ui, sans-serif;
  }
  .action {
    font-size: 11px;
    padding: 2px 6px;
  }
  .dismiss {
    width: 22px;
    height: 22px;
    font-size: 14px;
    line-height: 1;
  }
  .action:hover,
  .action:focus-visible,
  .dismiss:hover,
  .dismiss:focus-visible {
    background: rgba(255, 255, 255, 0.12);
    outline: 2px solid #4ea1ff;
  }
</style>
