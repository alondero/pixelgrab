<script lang="ts">
  import { onMount } from "svelte";
  import type { RevisionContext } from "$lib/ipc/types";
  import ShelfCard from "./ShelfCard.svelte";
  import { createShelfController } from "./controller.svelte";
  let { onEdit }: { onEdit: (context: RevisionContext) => void } = $props();
  const shelf = createShelfController((context) => onEdit(context));
  onMount(() => {
    void shelf.start();
    return () => shelf.dispose();
  });
</script>

<section aria-label="Recent screenshots" class="recent">
  <div class="heading">
    <h2>Recent screenshots</h2>
    <span>{shelf.cards.length}</span>
  </div>
  <p class="hint">Drag any image into another app. Click an image to open it.</p>
  {#if shelf.loading}
    <p role="status">Loading screenshots…</p>
  {:else if shelf.cards.length === 0}
    <div class="empty">
      <strong>Your next screenshot will appear here.</strong>
      <p>Capture an area, release the mouse, then drag the preview into your app.</p>
    </div>
  {:else}
    <div class="grid">
      {#each shelf.cards as card (card.shelfId)}
        <ShelfCard
          {card}
          nowMs={shelf.snapshot?.snapshotAtMs ?? 0}
          showCountdown={false}
          {...shelf.actions}
        />
      {/each}
    </div>
  {/if}
  <p role="status" aria-live="polite" class="feedback" data-kind={shelf.feedback.message?.kind}>
    {shelf.feedback.message?.text ?? ""}
  </p>
  {#if shelf.feedback.message?.kind === "error" && shelf.cards.length === 0}
    <button type="button" onclick={() => shelf.start()}>Try again</button>
  {/if}
</section>

<style>
  .heading {
    display: flex;
    align-items: center;
    gap: 0.7rem;
  }
  h2 {
    font-size: 1.15rem;
    margin: 0;
  }
  .heading span {
    color: #5c6375;
    font-size: 0.85rem;
  }
  .hint {
    color: #626a7b;
    font-size: 0.9rem;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 16px;
  }
  .grid :global(.card) {
    width: 100%;
    height: 280px;
  }
  .empty {
    padding: 2rem;
    border: 1px dashed #b9c3d5;
    border-radius: 12px;
    text-align: center;
    color: #626a7b;
  }
  .empty strong {
    color: #252d41;
  }
  .feedback {
    min-height: 1.3em;
    font-size: 0.9rem;
  }
  .feedback[data-kind="error"] {
    color: #a51b36;
  }
</style>
