<script lang="ts">
  import type { ShelfQueueCard } from "$lib/ipc/types";
  import { convertFileSrc as tauriConvertFileSrc } from "@tauri-apps/api/core";
  import { formatRemaining, remainingMs } from "./queue.svelte";

  // One shelf card. Renders the thumbnail, the editable title, the
  // countdown, and the quick-action buttons. The parent component
  // (ShelfQueue) wires the `on*` callbacks to IPC commands.
  let {
    card,
    nowMs,
    showCountdown = true,
    onCopy = () => {},
    onSaveAs = () => {},
    onDismiss = () => {},
    onHover = () => {},
    onUnhover = () => {},
  }: {
    card: ShelfQueueCard;
    nowMs: number;
    showCountdown?: boolean;
    onCopy?: (shelfId: string) => void;
    onSaveAs?: (shelfId: string) => void;
    onDismiss?: (shelfId: string) => void;
    onHover?: (shelfId: string) => void;
    onUnhover?: (shelfId: string) => void;
  } = $props();

  type TauriGlobal = {
    core?: { convertFileSrc?: (p: string) => string };
  };
  function getTauri(): TauriGlobal | undefined {
    return (globalThis as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
  }
  function convertFileSrc(p: string): string {
    // Tauri 2 does not expose `__TAURI__.core` unless the global API is
    // explicitly enabled. Use the typed module API in packaged builds and
    // retain the raw path fallback for unit tests / non-Tauri previews.
    try {
      return tauriConvertFileSrc(p);
    } catch {
      return getTauri()?.core?.convertFileSrc?.(p) ?? p;
    }
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
  <div class="thumbnail" data-testid="shelf-thumbnail">
    <img src={pngUrl} alt={titleText} />
  </div>
  <div class="meta">
    <div class="title" data-testid="shelf-title">{titleText}</div>
    <div class="row">
      <span class="size" data-testid="shelf-size">
        {Math.round(card.sizeBytes / 1024)} KB
      </span>
      {#if showCountdown}
        <span
          class="countdown"
          class:paused
          class:expired
          data-testid="shelf-countdown"
          aria-label={paused ? "Card timer paused" : "Card timer remaining"}
        >
          {countdownText}
        </span>
      {/if}
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
    grid-template-columns: 56px 1fr;
    grid-template-rows: 1fr auto;
    grid-template-areas:
      "thumbnail meta"
      "actions actions";
    gap: 6px;
    padding: 6px 8px;
    color: #fff;
    font-family: system-ui, sans-serif;
    box-sizing: border-box;
    overflow: hidden;
    transition: opacity 200ms ease;
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
