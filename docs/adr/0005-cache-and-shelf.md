# ADR-0005: Cache & one-card shelf

## Status

Accepted (tracer-07).

## Context

Tracer 02 established the commit pipeline: flatten the user's crop
once, write a PNG, publish a bitmap to the clipboard. Tracer 07 must
extend that into a **durable cache entry** that survives process
restarts and a **one-card shelf** that the user can see, dismiss, and
edit metadata for.

The acceptance criteria require:

1. Enter atomically persists a capture and editable metadata, copies
   it to the clipboard, and displays a protected shelf card.
2. The card is the bottom-right of the primary monitor's **work area**
   (not its full bounds — the Windows taskbar must be respected).
3. A failed commit leaves neither a visible card nor an untracked
   partial entry.
4. A visible card protects its backing assets from deletion.

Without deliberate structure for these requirements the commit pipeline
ends up with three failure modes:

- **Orphan PNG.** A platform-level PNG write succeeds but the manifest
  never lands; the user has a PNG on disk with no way to find it.
- **Phantom card.** A cache entry is published to the shelf but the
  PNG is missing or corrupt; the user clicks the card and sees a
  broken thumbnail.
- **Premature cleanup.** A card is on screen but the LRU pruner (or a
  manual cleanup command) deletes its backing assets while the user
  is still interacting with it.

## Decision

We adopt a two-phase commit pipeline with a **manifest sentinel**:

```
<cache_root>/<capture_id>/
  capture.png       # Phase 1: flattened PNG, atomically renamed
  bitmap.png        # Phase 1: optional staging bitmap
  metadata.json     # Phase 1: editable metadata, atomically renamed
  manifest.json     # Phase 2: publish sentinel, written last
```

Every Phase-1 file is written via `atomic::write_atomic` (tmp + fsync +
rename). The manifest is the **only** signal the shelf consumes: the
shelf enumerates the cache root and considers an entry durable iff
its `manifest.json` exists. A partial entry (assets present, manifest
absent) is a crashed commit and is reaped by `Cache::load_or_recover`
on the next startup scan.

### Active locks

Each entry carries a typed set of active locks (`LockOwner::{Shelf,
Editor, Drag, Pin}`). `Cache::commit` always acquires a `Shelf` lock;
subsequent operations acquire additional locks as needed. The cache
stores the `Shelf` lock guard inside its own state so the lock lives
for the lifetime of the card; only `Cache::dismiss` releases it.
Cleanup is rejected while any owner holds the lock.

### Placement

The shelf window is positioned by `ShelfPosition::inside_primary_work_area`,
which anchors the card to the bottom-right of the primary monitor's
**work area** (not its bounds). The 24 px inset is carried as a field
on the struct so tests can assert the policy and future revisions can
tweak it without recomputing every call site.

### Two-phase Enter

Pressing Enter (or Ctrl+C in tracer-02, unified here) runs `commit`
in `src-tauri/src/ipc/commands.rs`:

1. `flatten_crop` is the single source of truth for the flattened
   RGBA. Both the on-disk PNG and the clipboard bitmap derive from the
   same buffer.
2. `Cache::commit` writes assets atomically, then writes the
   manifest.
3. On success the shelf window is positioned and shown, the
   `pixelgrab://shelf-updated` event is emitted to the frontend, and
   a `Shelf` lock guard is held inside the cache.
4. The session is transitioned `Committing -> Cleanup -> Idle`.

On any failure: no card is shown, no event is emitted, no entry is
left in the in-memory map, and the partial directory is reaped on the
next startup scan.

## Consequences

### Positive

- The shelf is a single source of truth for the user's most recent
  commit. There is exactly one card on screen at a time.
- A failed commit is a no-op from the user's perspective — they see
  no card, no missing PNG, no orphaned entry.
- Restart after a partial commit is automatic. The user does not need
  to know that recovery happened.
- Lock owners are typed and exhaustively listed. Adding a new owner
  is a one-line enum variant; the compiler enforces every match site.

### Negative

- The two-phase commit adds one synchronous filesystem operation
  (manifest write) to the commit path. The cost is dominated by the
  PNG write, so the total cost is unchanged to within ~1 %.
- The cache stores one `LockGuard` per entry in memory. For a one-card
  shelf this is one guard per entry; the memory cost is negligible.

### Trade-offs

- We accept the extra manifest write in exchange for the recovery
  guarantee.
- We accept the typed enum in exchange for an exhaustive list of
  consumers that must release the lock before cleanup.

## Alternatives

- **Loose PNG, no manifest.** Rejected. Cannot distinguish durable
  entries from crashed commits; recovery is impossible.
- **Database-backed cache (sqlite).** Rejected. Adds a native
  dependency, complicates the Windows packaging story, and provides
  no benefit for a one-card shelf where the entry count is bounded.
- **Single `entries.json` file.** Rejected. The atomic-write story
  becomes complex (write-then-rename-of-the-whole-file), and the
  shelf would have to parse the whole file on every commit.
- **Generic reference-counted locks (`Arc::strong_count`).** Rejected.
  Locks should be released explicitly when consumers (drag, editor)
  finish, not when the last `Arc` happens to be dropped. A typed enum
  makes the policy auditable.
