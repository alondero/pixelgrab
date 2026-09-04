// Shelf window entrypoint. Mounts the ShelfQueue component and
// listens for `pixelgrab://shelf-queue-updated` events from the
// Rust core. Quick actions (copy, save-as, dismiss, hover, unhover)
// are wired to the corresponding IPC commands. Visible success /
// error feedback for Copy and Save As is provided by an aria-live
// region inside the ShelfQueue component, fed by a `feedback`
// reactive store.
//
// Tracer 15 (closes #34): on startup the shelf is **event-only**,
// so a restart while the cache already holds entries renders an
// empty queue until the next commit fires an event. We now seed
// `currentSnapshot` from `get_shelf_queue_snapshot` during init
// and let subsequent events overwrite it. The rehydration runs as
// a fire-and-forget async so the mount remains synchronous; the
// promise is intentionally not awaited on the module path so an
// IPC failure cannot block the shelf window from appearing.

import { mount } from "svelte";
import { listen, emit } from "@tauri-apps/api/event";
import ShelfQueue from "./lib/shelf/ShelfQueue.svelte";
import {
  copyShelfCard,
  dismissCacheEntry,
  getShelfPreferences,
  getShelfQueueSnapshot,
  hoverShelfCard,
  openRevision,
  saveShelfCardAs,
  showMainWindow,
  startShelfDrag,
  tickShelfQueue,
  unhoverShelfCard,
} from "./lib/ipc/commands";
import type {
  RevisionContext,
  ShelfPreferencesDto,
  ShelfQueueCard,
  ShelfQueueSnapshot,
} from "./lib/ipc/types";
import type { ShelfClearedEvent } from "./lib/shelf/types";
import { createFeedbackStore } from "./lib/shelf/feedback.svelte";
import { pinStore } from "./lib/pin/pinStore.svelte";

const target = document.getElementById("shelf");
if (!target) {
  throw new Error("shelf root element not found");
}

// Runes-mode reactive state: the component re-renders whenever
// `currentSnapshot` is reassigned.
let currentSnapshot = $state<ShelfQueueSnapshot | null>(null);
let showCountdown = $state(true);
let preferenceEventReceived = false;

// Visible feedback for quick actions. The ShelfQueue renders an
// `aria-live="polite"` region; the store is fed by the onCopy /
// onSaveAs callbacks below.
const feedback = createFeedbackStore();

function findCard(shelfId: string): ShelfQueueCard | undefined {
  const snapshot = currentSnapshot;
  if (!snapshot) return undefined;
  return (
    snapshot.cards.find((card) => card.shelfId === shelfId) ??
    snapshot.overflow.find((card) => card.shelfId === shelfId)
  );
}

function removeFromSnapshot(shelfId: string) {
  if (!currentSnapshot) return;
  const cards = currentSnapshot.cards.filter((card) => card.shelfId !== shelfId);
  const overflow = currentSnapshot.overflow.filter((card) => card.shelfId !== shelfId);
  // Keep the snapshot object when the id was not present. A stale cleared
  // event must never blank a queue that has already received a newer event.
  if (
    cards.length === currentSnapshot.cards.length &&
    overflow.length === currentSnapshot.overflow.length
  ) {
    return;
  }
  currentSnapshot = { ...currentSnapshot, cards, overflow };
}

mount(ShelfQueue, {
  target,
  props: {
    get snapshot() {
      return currentSnapshot;
    },
    get feedback() {
      return feedback.message;
    },
    get showCountdown() {
      return showCountdown;
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
    onDismiss: async (shelfId: string) => {
      // Hide the card immediately for responsive dismissal. The backend
      // event remains authoritative and rehydrates the queue if the delete
      // fails. A still-locked result is successful shelf dismissal: another
      // owner deliberately keeps the underlying cache entry alive.
      removeFromSnapshot(shelfId);
      const response = await dismissCacheEntry({ shelfId });
      if (response.status === "err") {
        const snapshot = await getShelfQueueSnapshot();
        if (snapshot.status === "ok") currentSnapshot = snapshot.data;
      }
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
    // Issue #63: pin the card into an independent TopMost reference
    // window. The Rust core creates the native window and acquires
    // the cache `Pin` lock; the frontend only supplies the source.
    onPin: async (shelfId: string) => {
      const card = findCard(shelfId);
      if (!card) return;
      const view = await pinStore.openPin({
        captureId: card.captureId,
        pngPath: card.pngPath,
        bounds: card.bounds,
      });
      if (view) {
        feedback.flash("Pinned to screen", "success");
      } else {
        feedback.flash("Pin failed", "error");
      }
    },
    // Issue #63: reopen the card for non-destructive editing. On
    // success the restored scene is forwarded to the main window via
    // `pixelgrab://revision-opened` and the companion window is shown
    // so the revision editor becomes visible.
    onEdit: async (card: ShelfQueueCard) => {
      const response = await openRevision({ shelfId: card.shelfId });
      if (response.status === "ok") {
        const context: RevisionContext = response.data.context;
        await emit("pixelgrab://revision-opened", context);
        await showMainWindow();
      } else {
        feedback.flash(`Reopen failed: ${response.error.message}`, "error");
      }
    },
    // Issue #63: start the native OLE drag. Only `Accepted` dismisses
    // the card, and the backend computes that policy — the frontend
    // just follows the returned hint.
    onDrag: async (card: ShelfQueueCard) => {
      const response = await startShelfDrag({
        shelfId: card.shelfId,
        dismissOnAccepted: true,
      });
      if (response.status === "ok") {
        if (response.data.shouldDismiss) {
          void dismissCacheEntry({ shelfId: card.shelfId });
        } else {
          feedback.flash("Drop cancelled — card kept", "info");
        }
      } else {
        feedback.flash(`Drag failed: ${response.error.message}`, "error");
      }
    },
  },
});

// Rehydrate the queue from the authoritative server state on
// startup so a restart while the cache already holds entries
// renders cards immediately, instead of waiting for the next
// `pixelgrab://shelf-queue-updated` event. The fire-and-forget
// pattern keeps the mount synchronous; a failing IPC is logged
// (the shelf window stays usable — the live event still fires on
// every subsequent update) but never blocks the window from
// appearing.
void (async () => {
  const [queueResponse, preferencesResponse] = await Promise.all([
    getShelfQueueSnapshot(),
    getShelfPreferences(),
  ]);
  if (queueResponse.status === "ok" && queueResponse.data) {
    currentSnapshot = queueResponse.data;
  } else if (queueResponse.status === "err") {
    // Tracer-15 review (Standards axis): the shelf must remain
    // visible even when rehydration fails, but the failure should
    // be observable so a regression in the IPC layer shows up in
    // devtools instead of an empty window with no diagnostic.
    console.warn("shelf rehydrate failed", queueResponse.error);
  }
  // A live preference event is newer than a bootstrap response that may have
  // been captured before the update. Never let the slow startup read roll it
  // back.
  if (preferencesResponse.status === "ok" && !preferenceEventReceived) {
    showCountdown = preferencesResponse.data.showCountdown;
  }
})();

listen<ShelfQueueSnapshot>("pixelgrab://shelf-queue-updated", (event) => {
  currentSnapshot = event.payload;
});

// Shelf History is an explicit navigation action, unlike the passive shelf
// reveal after a commit. Move focus to the newest card only for that action
// so keyboard users can immediately use C / Ctrl+C / Ctrl+S / P / Delete.
listen("pixelgrab://shelf-focus-requested", () => {
  requestAnimationFrame(() => {
    target.querySelector<HTMLElement>('[data-testid="shelf-card"]')?.focus();
  });
});

listen<ShelfPreferencesDto>("pixelgrab://shelf-preferences-updated", (event) => {
  preferenceEventReceived = true;
  showCountdown = event.payload.showCountdown;
});

// When the backend signals that the shelf is empty (e.g. after a
// dismissal) the card is hidden, not destroyed — Tauri's webview is
// cheap to keep alive.
listen<ShelfClearedEvent>("pixelgrab://shelf-cleared", (event) => {
  // The payload carries the cleared `shelfId` so future listeners
  // (analytics, focused per-card teardown) can correlate against the
  // queue snapshot that follows. This listener only needs the bare
  // trigger — the queue snapshot is the authoritative state — so
  // the field is intentionally unused here but kept in the typed
  // event to bind the wire shape (see `ShelfClearedEvent` in
  // `$lib/shelf/types` and the contract pair tests).
  void event.payload.shelfId;
  // The queue-updated event carries the authoritative remaining cards, and
  // the cleared event may arrive after it. Remove only the matching card;
  // clearing the whole snapshot made dismissing one card hide every card.
  removeFromSnapshot(event.payload.shelfId);
});
