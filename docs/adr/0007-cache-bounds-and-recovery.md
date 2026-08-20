# 0007 — Cache bounds + recovery (tracer-13)

## Status

Accepted (tracer-13).

## Context

The PixelGrab temporary capture cache introduced by tracer-07
(durable per-entry directory with a two-phase commit, manifest
sentinel, active lock registry) establishes "an entry survives a
crash" but says nothing about how much the cache is allowed to
grow, how old entries are allowed to become, or how leftover
debris from a crashed commit is reaped.

Issue #25 specifies the bounds (250 MB / 500 entries / 24 h) plus
a recovery sweep that grades the cache from a crash back to a
healthy state on the very next startup. The recovery must not
delay the tray becoming visible — a user who double-clicks the
tray during a long recovery should still see the app respond.

The cache already has the right primitives (the lock registry, the
two-phase commit, the in-memory `entries` map) so the new code is
a _policy_ layer on top of them, not a replacement.

## Decision

### Default policy

The tracer-13 defaults are pinned by the issue #25 spec:

- `max_bytes = 250 * 1024 * 1024` (250 MiB)
- `max_entries = 500`
- `max_age_ms = 24 * 60 * 60 * 1000` (24 h)
- `low_water_ratio = 0.8` (prune until at or below 80% of the
  high-water limits)
- `sweep_interval_ms = 15 * 60 * 1000` (15 min)
- `purge_on_exit = false`

The user can override any of these via the `update_cache_policy`
IPC. The Rust core sanitises the payload (every numeric field is
clamped to a documented range) so a tampered settings file can
never crash the app.

### Eviction algorithm

The sweep runs two stages in order:

1. **TTL**: every entry whose `last_access_at_ms` is older than
   `max_age_ms` is evicted, unless it is protected by a non-`Shelf`
   lock owner (editor / drag / pin).
2. **Quota**: after TTL, the oldest unlocked entries are evicted
   (LRU by `last_access_at_ms` ascending) until total bytes and
   entry count are both at or below the low-water targets.

The candidate list is snapshotted once at the start of the quota
loop so the per-iteration cost is O(n) instead of O(n²). The
sweeper breaks out of the loop on first lock race to avoid
overshooting the target.

### Lock semantics

The default `Shelf` lock is the marker every commit acquires and
does NOT protect from the sweeper. An entry is protected only when
it has any non-`Shelf` lock owner — editor (annotation in
progress), drag (OLE IDataObject alive), or pin (reference window
open). The helper `Cache::is_protected_from_sweeper` is the
single source of truth for this check; the stats summary, the
manual clear, and the sweeper all use it.

Rationale: the Shelf lock is a "this entry exists" marker, not a
"this entry is actively visible" signal. If the Shelf lock
protected, no entry would ever be evictable. The implementation
plan's "active shelf entries" wording maps to the shelf queue
engine's visible-set — the renderer pins a card via the Shelf
lock for the duration of its visibility — and the sweeper's intent
is the underlying cache survival, which is governed by the
explicit non-default owners.

### Recovery

The startup recovery runs on a `spawn_blocking` thread inside the
`setup` hook so the tray appears without waiting for the sweep to
finish. The recovery sweeps:

- Stale `*.tmp` files at the cache root (atomic-write leftovers).
- Zero-byte `capture.png` or `metadata.json` files inside entry
  directories.
- Empty entry directories (manifest present but no assets).
- Manifest-less directories (incomplete unindexed groups from a
  crashed commit).

The periodic worker calls `sweep_once` every `sweep_interval_ms`.
Both stages share the partial-failure semantics: when a per-file
error occurs (typically a permission error from a stray antivirus
scan), the sweep increments `partial_failures` and continues so
the rest of the cache can be reaped.

### Privacy

Error messages are categorical kind strings only — never raw file
paths. The cache policy root lives outside the cache root, so any
log would leak a path outside the cache. The `write_to_disk`
helper in `cache::policy.rs` follows the same pattern as the
shelf preferences store: the disk-io error kind is the category,
the `io::Error`'s `Display` (which can include the absolute path
on Windows) is discarded.

## Consequences

- New IPC surface: `get_cache_policy`, `update_cache_policy`,
  `get_cache_stats`, `clear_cache`. The frontend surfaces the
  settings in a new "Cache" tab of the settings panel.
- New background worker: `pixelgrab-cache-sweeper` thread spawned
  by `CacheSweeper::start_periodic`. Stopped on graceful shutdown.
- New on-disk file: `cache-policy.json` under
  `%LOCALAPPDATA%\com.pixelgrab.app\`. The file is next to
  `shelf-preferences.json` and uses the same atomic-write +
  backup-rotation + trailing-debounce shape.
- The hands-off `cache::store::recover_debris` must run before the
  sweeper's periodic worker starts so the cache is in a healthy
  state on the next periodic tick.
- The shelf queue engine's `touch_entries` is called when a card
  is hovered or shown so the LRU order tracks the user's actual
  attention rather than the wall-clock time the entry was first
  committed.

## Supersedes

None. ADR-0005 (Cache and one-card shelf) is extended by this ADR
but its durability, atomic-write, and lock-registry invariants
remain authoritative.
