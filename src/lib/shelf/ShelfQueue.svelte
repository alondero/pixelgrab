<script lang="ts">
  import type { ShelfQueueCard, ShelfQueueSnapshot } from "$lib/ipc/types";
  import ShelfCard from "./ShelfCard.svelte";
  import { createClockStore } from "./queue.svelte";
  import type { FeedbackEntry } from "./feedback.svelte";

  // The shelf queue. Renders up to four cards side-by-side with an
  // expandable "+N" overflow group for older captures. The component
  // subscribes to `nowMs` via `requestAnimationFrame` so each card's
  // countdown updates smoothly without a backend round-trip. The
  // optional `feedback` prop surfaces quick-action success / error
  // messages in an `aria-live="polite"` region.
  let {
    snapshot,
    feedback = null,
    onCopy = () => {},
    onSaveAs = () => {},
    onDismiss = () => {},
    onHover = () => {},
    onUnhover = () => {},
    onTickExpired = () => {},
    onPin = () => {},
    onEdit = () => {},
    onDrag = () => {},
  }: {
    snapshot: ShelfQueueSnapshot | null;
    feedback?: FeedbackEntry | null;
    onCopy?: (shelfId: string) => void;
    onSaveAs?: (shelfId: string) => void;
    onDismiss?: (shelfId: string) => void;
    onHover?: (shelfId: string) => void;
    onUnhover?: (shelfId: string) => void;
    onTickExpired?: () => void;
    onPin?: (shelfId: string) => void;
    onEdit?: (card: ShelfQueueCard) => void;
    onDrag?: (card: ShelfQueueCard) => void;
  } = $props();

  let clock = createClockStore();
  let overflowOpen = $state(false);

  // Detect cards that just expired locally so we can notify the
  // backend (the authoritative dismiss happens server-side via the
  // background ticker AND the rAF-driven IPC). The `reportedExpired`
  // set remembers which shelves we already reported so a card that
  // lingers in the DOM for one frame after expiry doesn't fire twice.
  let reportedExpired = $state(new Set<string>());

  $effect(() => {
    if (!snapshot) return;
    const now = clock.nowMs;
    const newly: string[] = [];
    for (const card of [...snapshot.cards, ...snapshot.overflow]) {
      const isPaused = card.timer.pausedAtElapsedMs !== undefined;
      const isExpired = !isPaused && now >= card.timer.deadlineAtElapsedMs;
      if (isExpired && !reportedExpired.has(card.shelfId)) {
        newly.push(card.shelfId);
      }
    }
    if (newly.length > 0) {
      const next = new Set(reportedExpired);
      for (const id of newly) next.add(id);
      reportedExpired = next;
      onTickExpired();
    }
    // Garbage-collect ids that are no longer in the queue so the set
    // does not grow unboundedly across many commits.
    const live = new Set([...snapshot.cards, ...snapshot.overflow].map((c) => c.shelfId));
    if ([...reportedExpired].some((id) => !live.has(id))) {
      const next = new Set<string>();
      for (const id of reportedExpired) {
        if (live.has(id)) next.add(id);
      }
      reportedExpired = next;
    }
  });

  // Stop the rAF clock loop when the queue empties so the shelf
  // window does not leak requestAnimationFrame handles between
  // sessions.
  $effect(() => {
    if (!snapshot || (snapshot.cards.length === 0 && snapshot.overflow.length === 0)) {
      clock.stop();
    } else {
      clock.start();
    }
  });

  let hasCards = $derived(
    !!snapshot && (snapshot.cards.length > 0 || snapshot.overflow.length > 0),
  );
  let overflowCount = $derived(snapshot?.overflow.length ?? 0);
</script>

{#if snapshot && hasCards}
  <div class="queue" data-testid="shelf-queue">
    {#each snapshot.cards as card (card.shelfId)}
      <ShelfCard
        {card}
        nowMs={clock.nowMs}
        {onCopy}
        {onSaveAs}
        {onDismiss}
        {onHover}
        {onUnhover}
        {onPin}
        {onEdit}
        {onDrag}
      />
    {/each}
    {#if overflowCount > 0}
      <div class="overflow" data-testid="shelf-overflow">
        <button
          type="button"
          class="overflow-toggle"
          aria-expanded={overflowOpen}
          aria-label="Show {overflowCount} older capture{overflowCount === 1 ? '' : 's'}"
          onclick={() => (overflowOpen = !overflowOpen)}
        >
          +{overflowCount}
        </button>
        {#if overflowOpen}
          <div class="overflow-panel" role="region" aria-label="Older captures">
            {#each snapshot.overflow as card (card.shelfId)}
              <ShelfCard
                {card}
                nowMs={clock.nowMs}
                {onCopy}
                {onSaveAs}
                {onDismiss}
                {onHover}
                {onUnhover}
                {onPin}
                {onEdit}
                {onDrag}
              />
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<div
  class="status"
  data-testid="shelf-feedback"
  data-kind={feedback?.kind ?? ""}
  role="status"
  aria-live="polite"
  aria-atomic="true"
>
  {feedback?.text ?? ""}
</div>

<style>
  .queue {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    background: transparent;
    color: #fff;
    font-family: system-ui, sans-serif;
  }
  .overflow {
    position: relative;
  }
  .overflow-toggle {
    width: 56px;
    height: 150px;
    background: rgba(28, 28, 32, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 8px;
    color: #fff;
    font-size: 16px;
    font-weight: 600;
    cursor: pointer;
    font-family: system-ui, sans-serif;
  }
  .overflow-toggle:hover,
  .overflow-toggle:focus-visible {
    background: rgba(28, 28, 32, 0.98);
    outline: 2px solid #4ea1ff;
  }
  .overflow-panel {
    position: absolute;
    right: 0;
    bottom: 158px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: transparent;
  }
  .status {
    position: absolute;
    right: 8px;
    top: 8px;
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 11px;
    font-family: system-ui, sans-serif;
    color: #fff;
    background: rgba(28, 28, 32, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.16);
    min-height: 18px;
    min-width: 12px;
    opacity: 0.95;
  }
  .status[data-kind="success"] {
    border-color: rgba(120, 220, 140, 0.6);
  }
  .status[data-kind="error"] {
    border-color: rgba(255, 122, 122, 0.7);
    color: #ff9c9c;
  }
  .status[data-kind="info"] {
    border-color: rgba(78, 161, 255, 0.6);
  }
</style>
