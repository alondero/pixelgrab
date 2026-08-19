// Shelf window entrypoint. Mounts the ShelfCard component and listens
// for `pixelgrab://shelf-updated` events from the Rust core.

import { mount } from "svelte";
import { listen } from "@tauri-apps/api/event";
import ShelfCard from "./lib/shelf/ShelfCard.svelte";
import { dismissCacheEntry } from "./lib/ipc/commands";
import type { ShelfCardView } from "./lib/shelf/types";

const target = document.getElementById("shelf");
if (!target) {
  throw new Error("shelf root element not found");
}

// Runes-mode reactive state: the component re-renders whenever
// `currentCard` is reassigned.
let currentCard = $state<ShelfCardView | null>(null);

mount(ShelfCard, {
  target,
  props: {
    get card() {
      return currentCard;
    },
    onDismiss: (shelfId: string) => {
      void dismissCacheEntry({ shelfId });
    },
  },
});

listen<ShelfCardView>("pixelgrab://shelf-updated", (event) => {
  currentCard = event.payload;
});

// When the backend signals that the shelf is empty (e.g. after a
// dismissal) the card is hidden, not destroyed — Tauri's webview is
// cheap to keep alive.
listen<{ shelfId: string }>("pixelgrab://shelf-cleared", (event) => {
  currentCard = null;
});
