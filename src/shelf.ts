// Shelf window entrypoint. Mounts the ShelfQueue component and
// listens for `pixelgrab://shelf-queue-updated` events from the
// Rust core. Quick actions (copy, save-as, dismiss, hover, unhover)
// are wired to the corresponding IPC commands. Visible success /
// error feedback for Copy and Save As is provided by an aria-live
// region inside the ShelfQueue component, fed by a `feedback`
// reactive store.

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
import { createFeedbackStore } from "./lib/shelf/feedback.svelte";

const target = document.getElementById("shelf");
if (!target) {
  throw new Error("shelf root element not found");
}

// Runes-mode reactive state: the component re-renders whenever
// `currentSnapshot` is reassigned.
let currentSnapshot = $state<ShelfQueueSnapshot | null>(null);

// Visible feedback for quick actions. The ShelfQueue renders an
// `aria-live="polite"` region; the store is fed by the onCopy /
// onSaveAs callbacks below.
const feedback = createFeedbackStore();

mount(ShelfQueue, {
  target,
  props: {
    get snapshot() {
      return currentSnapshot;
    },
    get feedback() {
      return feedback.message;
    },
    onCopy: async (shelfId: string) => {
      const response = await copyShelfCard({ shelfId });
      if (response.status === "ok") {
        feedback.flash("Copied to clipboard", "success");
      } else {
        feedback.flash(`Copy failed: ${response.error.message}`, "error");
      }
    },
    onSaveAs: async (shelfId: string) => {
      const response = await saveShelfCardAs({ shelfId });
      if (response.status === "ok") {
        if (response.data.path) {
          feedback.flash(`Saved to ${response.data.path}`, "success");
        } else {
          feedback.flash("Save cancelled", "info");
        }
      } else {
        feedback.flash(`Save failed: ${response.error.message}`, "error");
      }
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
