// TypeScript mirror of the Rust shelf queue contracts. The wire
// shapes are camelCase on both sides; the contract tests in
// `src/lib/ipc/types.test.ts` and
// `crates/pixelgrab-contracts/src/shelf_queue.rs` verify the
// round-trip stays in sync.
//
// Tracer 08 extends the shelf from one card to a queue of up to
// four visible cards plus an expandable `+N` overflow group, with
// per-card timers that pause on hover.

import type {
  ShelfPosition,
  ShelfQueueCard,
  ShelfQueueSnapshot,
  ShelfTimerConfig,
  ShelfTimerState,
} from "$lib/ipc/types";

export type {
  ShelfPosition,
  ShelfQueueCard,
  ShelfQueueSnapshot,
  ShelfTimerConfig,
  ShelfTimerState,
};

// Legacy single-card view kept for tracer-07 callers. The queue
// snapshot supersedes it everywhere in tracer 08.
export interface ShelfCardView {
  shelfId: string;
  captureId: string;
  pngPath: string;
  sizeBytes: number;
  createdAtMs: number;
  bounds: import("$lib/ipc/types").PhysicalBounds;
  metadata: import("$lib/ipc/types").CacheEntryMetadata;
}
