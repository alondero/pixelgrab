<script lang="ts">
  import type { ShelfCardView } from "./types";

  // The shelf card renders one committed capture. The Rust core emits
  // `pixelgrab://shelf-updated` after every successful commit; this
  // component subscribes via the `card` prop in `src/shelf.ts`.
  let {
    card,
    onDismiss = () => {},
  }: { card: ShelfCardView | null; onDismiss?: (shelfId: string) => void } = $props();

  // Tauri's `convertFileSrc` turns an absolute filesystem path into a
  // webview-loadable URL. The shelf PNG is read via the local asset
  // protocol because CSP permits `asset:` for images.
  // The `__TAURI__` global is only present inside a Tauri webview; in
  // tests we fall back to the raw path (the test environment renders a
  // relative URL placeholder, never a real desktop asset).
  type TauriGlobal = {
    core?: { convertFileSrc?: (p: string) => string };
  };
  function getTauri(): TauriGlobal | undefined {
    return (globalThis as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
  }
  function convertFileSrc(p: string): string {
    return getTauri()?.core?.convertFileSrc?.(p) ?? p;
  }

  let pngUrl = $derived(card ? convertFileSrc(card.pngPath) : null);
  let titleText = $derived(card?.metadata.title?.trim() || "Untitled capture");
</script>

{#if card}
  <div
    class="card"
    data-testid="shelf-card"
    data-shelf-id={card.shelfId}
    aria-label="Shelf card: {titleText}"
    role="region"
  >
    <div class="thumbnail" data-testid="shelf-thumbnail">
      {#if pngUrl}
        <img src={pngUrl} alt={titleText} />
      {/if}
    </div>
    <div class="meta">
      <div class="title" data-testid="shelf-title">{titleText}</div>
      <div class="size" data-testid="shelf-size">
        {Math.round(card.sizeBytes / 1024)} KB
      </div>
    </div>
    <button
      type="button"
      class="dismiss"
      data-testid="shelf-dismiss"
      aria-label="Dismiss shelf card"
      onclick={() => onDismiss(card!.shelfId)}
    >
      ×
    </button>
  </div>
{/if}

<style>
  .card {
    width: 100%;
    height: 100%;
    background: rgba(28, 28, 32, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 8px;
    display: grid;
    grid-template-columns: 64px 1fr 24px;
    gap: 8px;
    padding: 8px;
    color: #fff;
    font-family: system-ui, sans-serif;
    box-sizing: border-box;
    overflow: hidden;
  }
  .thumbnail {
    width: 64px;
    height: 64px;
    background: #000;
    border-radius: 4px;
    overflow: hidden;
    align-self: center;
  }
  .thumbnail img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .meta {
    align-self: center;
    min-width: 0;
  }
  .title {
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size {
    font-size: 10px;
    opacity: 0.7;
    margin-top: 2px;
  }
  .dismiss {
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.24);
    color: #fff;
    width: 24px;
    height: 24px;
    border-radius: 12px;
    align-self: flex-start;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }
  .dismiss:hover,
  .dismiss:focus-visible {
    background: rgba(255, 255, 255, 0.12);
    outline: 2px solid #4ea1ff;
  }
</style>
