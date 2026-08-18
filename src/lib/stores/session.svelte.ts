// Svelte 5 rune-based session store. The store is the single source of truth
// for the UI's view of the capture session. Components read it via the
// exported `session` object and call the IPC commands to mutate it.

import type { SessionSnapshot } from "$lib/ipc/types";

interface SessionStore {
  snapshot: SessionSnapshot;
  isCapturing: boolean;
  isSelecting: boolean;
}

function createSessionStore() {
  const inner: SessionStore = $state({
    snapshot: { state: "idle" },
    isCapturing: false,
    isSelecting: false,
  });

  return {
    get snapshot() {
      return inner.snapshot;
    },
    get isCapturing() {
      return inner.snapshot.state === "capturing";
    },
    get isSelecting() {
      return inner.snapshot.state === "selecting";
    },
    setSnapshot(next: SessionSnapshot) {
      inner.snapshot = next;
    },
    reset() {
      inner.snapshot = { state: "idle" };
    },
  };
}

export const session = createSessionStore();
