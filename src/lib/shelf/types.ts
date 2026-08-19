// TypeScript mirror of `src-tauri/src/shelf/mod.rs::ShelfCardView`. The
// wire shape is camelCase on both sides; the contract tests in
// `src/lib/ipc/types.test.ts` and `src-tauri/tests/ipc_contracts.rs`
// verify the round-trip stays in sync.

import type { CacheEntryMetadata, PhysicalBounds } from "$lib/ipc/types";

export interface ShelfCardView {
  shelfId: string;
  captureId: string;
  pngPath: string;
  sizeBytes: number;
  createdAtMs: number;
  bounds: PhysicalBounds;
  metadata: CacheEntryMetadata;
}
