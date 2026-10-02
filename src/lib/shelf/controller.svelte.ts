import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  copyShelfCard,
  dismissCacheEntry,
  getShelfPreferences,
  getShelfQueueSnapshot,
  hoverShelfCard,
  openRevision,
  saveShelfCardAs,
  startShelfDrag,
  tickShelfQueue,
  unhoverShelfCard,
} from "$lib/ipc/commands";
import type {
  RevisionContext,
  ShelfPreferencesDto,
  ShelfQueueCard,
  ShelfQueueSnapshot,
} from "$lib/ipc/types";
import { pinStore } from "$lib/pin/pinStore.svelte";
import { createFeedbackStore } from "./feedback.svelte";

/** Shared recent-capture actions and subscriptions for both native windows. */
export function createShelfController(onEdit?: (context: RevisionContext) => void) {
  let snapshot = $state<ShelfQueueSnapshot | null>(null);
  let showCountdown = $state(false);
  let loading = $state(true);
  let queueEventReceived = false;
  let preferenceEventReceived = false;
  let disposed = false;
  let subscribed = false;
  const unlisteners: UnlistenFn[] = [];
  const feedback = createFeedbackStore();
  const dragging = new Set<string>();

  function remove(shelfId: string) {
    if (!snapshot) return;
    snapshot = {
      ...snapshot,
      cards: snapshot.cards.filter((c) => c.shelfId !== shelfId),
      overflow: snapshot.overflow.filter((c) => c.shelfId !== shelfId),
    };
  }
  async function subscribe<T>(name: string, handler: (payload: T) => void) {
    const unlisten = await listen<T>(name, (event) => {
      if (!disposed) handler(event.payload);
    });
    if (disposed) unlisten();
    else unlisteners.push(unlisten);
  }
  async function start() {
    try {
      if (!subscribed) {
        subscribed = true;
        const registrations = await Promise.allSettled([
          subscribe<ShelfQueueSnapshot>("pixelgrab://shelf-queue-updated", (next) => {
            queueEventReceived = true;
            snapshot = next;
          }),
          subscribe<{ shelfId: string }>("pixelgrab://shelf-cleared", (event) => {
            queueEventReceived = true;
            remove(event.shelfId);
          }),
          subscribe<ShelfPreferencesDto>("pixelgrab://shelf-preferences-updated", (prefs) => {
            preferenceEventReceived = true;
            showCountdown = prefs.autoDismissEnabled && prefs.showCountdown;
          }),
        ]);
        if (registrations.some((result) => result.status === "rejected")) {
          for (const unlisten of unlisteners.splice(0)) unlisten();
          subscribed = false;
          throw new Error("Shelf subscription unavailable");
        }
      }
      if (disposed) return;
      const [queue, prefs] = await Promise.all([getShelfQueueSnapshot(), getShelfPreferences()]);
      if (disposed) return;
      if (queue.status === "ok" && !queueEventReceived) snapshot = queue.data;
      else if (queue.status === "err" && !queueEventReceived)
        feedback.flash("Could not load recent screenshots. Try again.", "error");
      if (prefs.status === "ok" && !preferenceEventReceived)
        showCountdown = prefs.data.autoDismissEnabled && prefs.data.showCountdown;
    } catch {
      if (!disposed) feedback.flash("Could not load recent screenshots. Try again.", "error");
    } finally {
      loading = false;
    }
  }

  const actions = {
    onCopy: async (shelfId: string) => {
      const response = await copyShelfCard({ shelfId });
      feedback.flash(
        response.status === "ok" ? "Copied to clipboard" : `Copy failed: ${response.error.message}`,
        response.status === "ok" ? "success" : "error",
      );
    },
    onSaveAs: async (shelfId: string) => {
      const response = await saveShelfCardAs({ shelfId });
      if (response.status === "err")
        feedback.flash(`Save failed: ${response.error.message}`, "error");
      else
        feedback.flash(
          response.data.path ? "Screenshot saved" : "Save cancelled",
          response.data.path ? "success" : "info",
        );
    },
    onDismiss: async (shelfId: string) => {
      const response = await dismissCacheEntry({ shelfId });
      if (response.status === "ok") remove(shelfId);
      else feedback.flash(`Dismiss failed: ${response.error.message}`, "error");
    },
    onHover: (shelfId: string) => {
      void hoverShelfCard({ shelfId }).catch(() => {});
    },
    onUnhover: (shelfId: string) => {
      if (!dragging.has(shelfId)) void unhoverShelfCard({ shelfId }).catch(() => {});
    },
    onTickExpired: () => {
      void tickShelfQueue().catch(() => {});
    },
    onPin: async (shelfId: string) => {
      const card = snapshot?.cards.concat(snapshot.overflow).find((c) => c.shelfId === shelfId);
      if (!card) return;
      const view = await pinStore.openPin({
        captureId: card.captureId,
        pngPath: card.pngPath,
        bounds: card.bounds,
      });
      feedback.flash(view ? "Pinned to screen" : "Pin failed", view ? "success" : "error");
    },
    onEdit: async (card: ShelfQueueCard) => {
      const response = await openRevision({ shelfId: card.shelfId });
      if (response.status === "err") {
        feedback.flash(`Open failed: ${response.error.message}`, "error");
        return;
      }
      if (onEdit) onEdit(response.data.context);
    },
    onDrag: async (card: ShelfQueueCard) => {
      if (dragging.has(card.shelfId)) return;
      dragging.add(card.shelfId);
      try {
        // A target can return from OLE before it has read CF_HDROP's file.
        // Keep the screenshot and its shelf lock for reuse and delayed readers.
        const response = await startShelfDrag({ shelfId: card.shelfId, dismissOnAccepted: false });
        if (response.status === "err")
          feedback.flash(`Drag failed: ${response.error.message}`, "error");
        else {
          const messages = {
            accepted: "Screenshot shared",
            cancelled: "Drop cancelled — screenshot kept",
            rejected: "This app did not accept the screenshot. Try Copy instead.",
            failed: "Drop failed. Try Copy instead.",
          } as const;
          feedback.flash(
            messages[response.data.outcome],
            response.data.outcome === "accepted"
              ? "success"
              : response.data.outcome === "cancelled"
                ? "info"
                : "error",
          );
        }
      } finally {
        dragging.delete(card.shelfId);
        void unhoverShelfCard({ shelfId: card.shelfId }).catch(() => {});
      }
    },
  };

  function handleAction<T>(message: string, action: (value: T) => Promise<void>) {
    return async (value: T) => {
      try {
        await action(value);
      } catch {
        feedback.flash(message, "error");
      }
    };
  }

  return {
    get snapshot() {
      return snapshot;
    },
    get cards() {
      return snapshot?.cards.concat(snapshot.overflow) ?? [];
    },
    get showCountdown() {
      return showCountdown;
    },
    get loading() {
      return loading;
    },
    feedback,
    actions: {
      ...actions,
      onCopy: handleAction("Could not copy the screenshot. Try again.", actions.onCopy),
      onSaveAs: handleAction("Could not save the screenshot. Try again.", actions.onSaveAs),
      onDismiss: handleAction("Could not dismiss the screenshot. Try again.", actions.onDismiss),
      onPin: handleAction("Could not pin the screenshot. Try again.", actions.onPin),
      onEdit: handleAction("Could not open the screenshot. Try again.", actions.onEdit),
      onDrag: handleAction("Could not share the screenshot. Try Copy instead.", actions.onDrag),
    },
    start,
    dispose() {
      disposed = true;
      feedback.clear();
      for (const fn of unlisteners) fn();
    },
  };
}
