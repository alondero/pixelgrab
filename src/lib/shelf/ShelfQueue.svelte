<script lang="ts">
  import type { ShelfQueueSnapshot } from "$lib/ipc/types";
  import ShelfCard from "./ShelfCard.svelte";
  import { createClockStore } from "./queue.svelte";

  // The shelf queue. Renders up to four cards side-by-side with an
  // expandable "+N" overflow group for older captures. The component
  // subscribes to `nowMs` via `requestAnimationFrame` so each card's
  // countdown updates smoothly without a backend round-trip.
  let {
    snapshot,
    onCopy = () => {},
    onSaveAs = () => {},
    onDismiss = () => {},
    onHover = () => {},
    onUnhover = () => {},
    onTickExpired = () => {},
  }: {
    snapshot: ShelfQueueSnapshot | null;
    onCopy?: (shelfId: string) => void;
    onSaveAs?: (shelfId: string) => void;
    onDismiss?: (shelfId: string) => void;
    onHover?: (shelfId: string) => void;
    onUnhover?: (shelfId: string) => void;
    onTickExpired?: () => void;
  } = $props();

  let clock = createClockStore();
  let overflowOpen = $state(false);

  // Detect cards that just expired locally so we can notify the
  // backend (the authoritative dismiss happens server-side). The
  // `lastSeenExpired` set remembers which shelves we already
  // reported so a card that lingers in the DOM for one frame after
  // expiry doesn't fire twice.
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

  let hasCards = $derived(
    !!snapshot && (snapshot.cards.length > 0 || snapshot.overflow.length > 0),
  );
  let overflowCount = $derived(snapshot?.overflow.length ?? 0);
</script>

{#if snapshot && hasCards}
  <div class="queue" data-testid="shelf-queue">
    {#each snapshot.cards as card (card.shelfId)}
      <ShelfCard {card} nowMs={clock.nowMs} {onCopy} {onSaveAs} {onDismiss} {onHover} {onUnhover} />
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
              />
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

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
</style>
