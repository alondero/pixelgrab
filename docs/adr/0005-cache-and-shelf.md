# ADR-0005: Cache & shelf queue

## Status

Accepted (tracer-07, extended by tracer-08).

## Context

Tracer 02 established the commit pipeline: flatten the user's crop
once, write a PNG, publish a bitmap to the clipboard. Tracer 07 must
extend that into a **durable cache entry** that survives process
restarts and a **shelf** that the user can see, dismiss, and edit
metadata for.

The acceptance criteria require:

1. Enter atomically persists a capture and editable metadata, copies
   it to the clipboard, and displays a protected shelf card.
2. The card is the bottom-right of the primary monitor's **work area**
   (not its full bounds — the Windows taskbar must be respected).
3. A failed commit leaves neither a visible card nor an untracked
   partial entry.
4. A visible card protects its backing assets from deletion.

Tracer 08 adds: the shelf must support **multiple recent captures at
once** with per-card hover-paused timers, an expandable `+N` overflow
group, and quick actions (Copy, Save As, Dismiss). The card lock must
be released only when the card leaves every shelf representation.

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

### Cache root and startup sequence

The on-disk root is resolved by `default_cache_root()` and wired
into the cache at `app` setup (`setup` hook in `src-tauri/src/lib.rs`):

- **Windows.** `<%LOCALAPPDATA%>\com.pixelgrab.app\cache` — the
  per-app directory created by the installer; the cache lives under
  `cache\` so it can be siblings with the preferences and policy
  files without collision.
- **Non-Windows / CI.** Falls back to `<temp>/pixelgrab-cache` so
  Linux / macOS dev builds and CI runs have a stable, writable home
  without the `LOCALAPPDATA` environment variable.

The startup sequence is two steps:

1. `Cache::set_cache_root(Some(root))` — creates the directory (or
   reuses it) and stores the path on the cache. The directory is
   created up-front so the first commit does not race with the
   mkdir.
2. `Cache::load_or_recover()` — scans the root, loads every entry
   with a `manifest.json`, and reaps partial entries (assets
   present, manifest absent). The scan is **synchronous but
   non-blocking in practice**: it is the only blocking step on the
   tray path, and it is bounded by the cache contents the user
   actually has.

A third pass runs on a worker thread (`SweepWorker::recover_startup`
in `cache::sweeper`, introduced in tracer-13) to reap `*.tmp`
debris, zero-byte assets, and empty entry directories without
touching valid active captures. The worker thread means the tray
appears before every byte of legacy debris is processed; the
periodic worker installed after the splash continues the same
sweep on a 15-minute cadence.

The startup scan is best-effort: a failed `set_cache_root` or
`load_or_recover` is logged at `warn` and the app still comes up so
the user can open a capture (a fresh, empty cache is the natural
fallback). The error message carries the cache root path so the
log is actionable without a second look-up.

### Active locks

Each entry carries a typed set of active locks (`LockOwner::{Shelf,
Editor, Drag, Pin}`). `Cache::commit` always acquires a `Shelf` lock;
subsequent operations acquire additional locks as needed. The cache
stores the `Shelf` lock guard inside its own state so the lock lives
for the lifetime of the card; only `Cache::dismiss` releases it.
Cleanup is rejected while any owner holds the lock.

The lock is released **only when the card is fully gone from every
shelf representation** (main view and overflow). Since both
representations are owned by the queue engine, an expiry from either
view runs through the queue's tick path, which hands the expired
shelf id back to the IPC layer so the cache can dismiss it. A manual
dismissal runs through `cache.dismiss` directly, which releases the
lock regardless of where the card was rendered.

### Placement

The shelf window is positioned by `ShelfPosition::shelf_queue_position`,
which anchors the row of cards to the bottom-right of the primary
monitor's **work area** (not its bounds) and scales the window width
with the visible card count. The 24 px inset is carried as a field
on the struct so tests can assert the policy and future revisions can
tweak it without recomputing every call site.

### Two-phase Enter

Pressing Enter (or Ctrl+C in tracer-02, unified here) runs `commit`
in `src-tauri/src/ipc/commands.rs`:

1. `flatten_crop` is the single source of truth for the flattened
   RGBA. Both the on-disk PNG and the clipboard bitmap derive from the
   same buffer. RGBA length is validated at the IPC boundary so a
   corrupt platform response never reaches either the clipboard or the
   cache.
2. The clipboard is published _first_. A clipboard failure aborts the
   commit before the cache is touched, so a clipboard error never
   leaves a phantom card.
3. `Cache::commit` writes assets atomically, then writes the
   manifest. Phase-1 or phase-2 failures reap the partial directory
   before the error is returned.
4. On success the new card is pushed onto the `ShelfQueueEngine`,
   the shelf window is repositioned for the new visible card count,
   and the `pixelgrab://shelf-queue-updated` event is emitted. A
   `Shelf` lock guard is held inside the cache.
5. `session.finish()` runs once at the end of every commit attempt
   (success or failure) so the session is always reset to `Idle`.

On any failure: no card is shown, no event is emitted, no entry is
left in the in-memory map, and the partial directory is reaped.

### Shelf queue engine

Tracer 08 introduced a dedicated `ShelfQueueEngine` that owns the
ordering, hover-pause, and timer state for the multi-card queue. The
engine mirrors the cache's entries but does **not** own their
persistence or locks — those stay with the cache. Every state
transition (commit → queue.add, dismiss → queue.dismiss + cache.dismiss,
tick → cache.dismiss per expired id) is driven by the IPC layer so
the cache and the queue can never drift out of lockstep.

Per-card timers use **monotonic elapsed millis** so a clock change
cannot reset a countdown mid-flight. Hover pauses the targeted card
only; the remaining time is captured at pause and re-applied at
un-hover with a **three-second grace bump** so a card with very
little remaining time still gets a fair chance to be read.

### Quick actions

The shelf card exposes three quick actions:

- **Copy.** The Rust core reads the cached PNG from disk and forwards
  it to `PixelGrabPlatform::publish_png_clipboard`. The default
  implementation decodes the PNG via the `png` crate and calls
  `publish_clipboard` so the same clipboard semantics apply whether
  the user commits a fresh crop or copies an older one. Windows
  implementations may override the default to write PNG bytes
  directly to the native clipboard.
- **Save As.** The Rust core opens the native Save As dialog via
  `tauri-plugin-dialog`, reads the PNG bytes, and writes them to the
  chosen path. The dialog runs on a worker thread so the async IPC
  future remains `'static`.
- **Dismiss.** The existing `dismiss_cache_entry` IPC. The queue
  removes the card and the cache releases the `Shelf` lock; the entry
  is reaped from disk when no other lock remains.

Expiry on the queue tick path returns the expired shelf ids to the
caller, who dismisses each from the cache so the lock is released
and the on-disk entry is reaped. A new shelf window position is
computed after every tick so a shrinking queue keeps the window
sized correctly.

## Consequences

### Positive

- The shelf is the user's queue of recent captures. Up to four cards
  are visible at once, with an expandable overflow group for older
  captures — nothing is hidden without an explicit action.
- A failed commit is a no-op from the user's perspective — they see
  no card, no missing PNG, no orphaned entry.
- Restart after a partial commit is automatic. The user does not need
  to know that recovery happened.
- Lock owners are typed and exhaustively listed. Adding a new owner
  is a one-line enum variant; the compiler enforces every match site.
- The queue engine owns ordering + timers; the cache owns
  durability + locks. The two stay in lockstep via the IPC layer,
  which keeps the lock invariant local to the cache module.
- Per-card timers use monotonic time and a grace bump so wall-clock
  changes and rapid hover-leave cycles cannot prematurely expire a
  card.

### Negative

- The two-phase commit adds one synchronous filesystem operation
  (manifest write) to the commit path. The cost is dominated by the
  PNG write, so the total cost is unchanged to within ~1 %.
- The cache stores one `LockGuard` per entry in memory. For a
  multi-card queue this is one guard per entry; the memory cost is
  still negligible (a handful of cards in the typical session).
- The shelf queue engine is a second source of list state. Every
  cache mutation must be mirrored via the queue, which is enforced
  by routing every cache event through the IPC layer.

### Trade-offs

- We accept the extra manifest write in exchange for the recovery
  guarantee.
- We accept the typed enum in exchange for an exhaustive list of
  consumers that must release the lock before cleanup.
- We accept the engine-cache mirror in exchange for keeping the
  cache focused on durability and the engine focused on UX state.

## Alternatives

- **Loose PNG, no manifest.** Rejected. Cannot distinguish durable
  entries from crashed commits; recovery is impossible.
- **Database-backed cache (sqlite).** Rejected. Adds a native
  dependency, complicates the Windows packaging story, and provides
  no benefit for a multi-card queue where the entry count is still
  bounded.
- **Single `entries.json` file.** Rejected. The atomic-write story
  becomes complex (write-then-rename-of-the-whole-file), and the
  shelf would have to parse the whole file on every commit.
- **Generic reference-counted locks (`Arc::strong_count`).** Rejected.
  Locks should be released explicitly when consumers (drag, editor)
  finish, not when the last `Arc` happens to be dropped. A typed enum
  makes the policy auditable.
- **Per-card webview windows.** Rejected. Tauri webviews are heavy;
  one shelf webview with a CSS grid rendering card slots is simpler
  and faster, and lets the Svelte side handle hover/leave
  deterministically without Tauri IPC per card.
- **Wall-clock-based timers.** Rejected. A user NTP-syncing their
  clock could watch a card instantly vanish. Monotonic elapsed millis
  make the timers robust against clock changes.
