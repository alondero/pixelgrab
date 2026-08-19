// Shelf window entrypoint. Mounts the ShelfQueue component and
// listens for `pixelgrab://shelf-queue-updated` events from the
// Rust core. Quick actions (copy, save-as, dismiss, hover, unhover)
// are wired to the corresponding IPC commands.

import { mount } from "svelte";
import { listen } from "@tauri-apps/api/event";
import ShelfQueue from "./lib/shelf/ShelfQueue.svelte";
import {
  copyShelfCard,
  dismissCacheEntry,
  hoverShelfCard,
  saveShelfCardAs,
  tickShelfQueue,
  unhoverShelfCard,
} from "./lib/ipc/commands";
import type { ShelfQueueSnapshot } from "./lib/ipc/types";

const target = document.getElementById("shelf");
if (!target) {
  throw new Error("shelf root element not found");
}

// Runes-mode reactive state: the component re-renders whenever
// `currentSnapshot` is reassigned.
let currentSnapshot = $state<ShelfQueueSnapshot | null>(null);

mount(ShelfQueue, {
  target,
  props: {
    get snapshot() {
      return currentSnapshot;
    },
    onCopy: (shelfId: string) => {
      void copyShelfCard({ shelfId });
    },
    onSaveAs: (shelfId: string) => {
      void saveShelfCardAs({ shelfId });
    },
    onDismiss: (shelfId: string) => {
      void dismissCacheEntry({ shelfId });
    },
    onHover: (shelfId: string) => {
      void hoverShelfCard({ shelfId });
    },
    onUnhover: (shelfId: string) => {
      void unhoverShelfCard({ shelfId });
    },
    onTickExpired: () => {
      void tickShelfQueue();
    },
  },
});

listen<ShelfQueueSnapshot>("pixelgrab://shelf-queue-updated", (event) => {
  currentSnapshot = event.payload;
});

// When the backend signals that the shelf is empty (e.g. after a
// dismissal) the card is hidden, not destroyed — Tauri's webview is
// cheap to keep alive.
listen<{ shelfId: string }>("pixelgrab://shelf-cleared", () => {
  // A cleared event is followed (or preceded) by a queue snapshot
  // update. Drop the local copy so the queue UI hides itself; the
  // authoritative state comes from the next snapshot.
  currentSnapshot = null;
});
