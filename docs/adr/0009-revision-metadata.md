# ADR-0009: Reopen / non-destructive revision metadata

## Status

Accepted (tracer-10, GitHub issue #22).

## Context

After tracer-07, every committed capture yielded a one-card shelf. The
tracer-04/05/06 annotations were baked into the shelf's PNG via
`flatten_annotations`, but the editor scene (annotations, badge
counter, tool / style state) was **not** persisted — once committed,
the user could not reopen the capture for further editing without
losing every annotation.

The acceptance criteria in issue #22 require:

1. Reopen a shelf card with all annotations, badges, styles, and
   z-order restored.
2. Cancel preserves the original assets and shelf state.
3. Commit creates a distinct new capture identity.
4. Failed commits cannot corrupt the original.
5. Missing / corrupt / older / future-version metadata degrades
   safely to flattened-image editing.
6. Lock ownership remains correct across open / cancel / failure /
   commit.
7. Badge numbering continues correctly across reopens.

## Decision

### 1. Sidecar `revision.json`

A new per-entry file lives next to `metadata.json` and `manifest.json`,
holding the editor scene:

```
<cache_root>/<capture_id>/
    capture.png       (frozen PNG, atomically renamed)
    metadata.json     (user-editable title / note / tags)
    revision.json     (NEW: editor scene, schema-versioned)
    manifest.json     (publish sentinel, written last)
```

The `revision.json` schema is versioned. The current version is
1 (`REVISION_SCHEMA_VERSION`). The loader is tolerant of unknown
fields (so additive serializer changes don't require a version bump)
but rejects any other version, surfacing a typed
`revision_unsupported_version` fall-back to the flat-PNG editor.

This separation keeps `metadata.json` (user-facing labels) and
`revision.json` (editor scene) independent: a future feature can
add new editor fields without touching the user label schema and
vice versa.

### 2. Lock ownership

The `LockOwner::Editor` variant has been declared since the cache
module was introduced (tracer-07) but no production code path
acquired it. Tracer-10 activates it.

| State                | Locks on source entry                          |
| -------------------- | ---------------------------------------------- |
| Idle (card on shelf) | `Shelf`                                        |
| Editing (reopen)     | `Shelf` + `Editor`                             |
| Commit in flight     | `Shelf` + `Editor`                             |
| Commit success       | `Shelf` (old) + `Shelf` (new) + Editor dropped |
| Cancel               | `Shelf` (Editor dropped)                       |

The `Editor` lock prevents the periodic sweeper (tracer-13) and
the manual `clear_cache` (tracer-13) from evicting the user's
work-in-progress. The `Shelf` lock keeps the original card visible
on the shelf throughout the reopen session.

The lock guard is owned by the cache (`Cache::editor_guards`,
mirroring `Cache::shelf_guards`) so its lifetime is tied to the
cache's mutex. The `Editor` lock is acquired by the cache layer
itself, not by the IPC handler, so the lock is dropped on the
dismiss path even when the IPC handler misses the release.

### 3. Revision commit = regular commit + new capture_id

The `commit_revision` IPC reuses `Cache::commit` for the new entry.
The new entry's `capture_id` and `shelf_id` are fresh UUIDs; the
source entry's `capture_id` / `shelf_id` are preserved. The new
entry sits next to the source in the cache root and is published
on the shelf queue via the same `shelf_queue.add` + `emit_shelf_queued_updated`
path as the regular commit.

The source entry's `revision.json` is updated with the in-progress
scene so a future reopen starts from the same point. The source
entry's `capture.png`, `metadata.json`, and `manifest.json` are
never touched by the commit path — the issue's "Cancellation does
not mutate original assets" guarantee.

The source entry's `metadata.json` is updated to reflect the
user's title / note / tags edits via the existing
`Cache::update_metadata` path. The on-disk `revision.json` also
carries the metadata so a reopen session starts with the same
author-visible labels.

### 4. Failure semantics

The IPC layer's `commit_revision` body wraps every side effect in a
closure so `session.finish_revision()` runs exactly once — mirroring
the existing tracer-07 round-2 fix for the "wedged session" bug.

- If the new entry's two-phase commit fails (PNG write, metadata
  write, manifest write), the partial entry is reaped by the
  existing two-phase commit invariant. The source entry's assets
  remain untouched.
- If the `write_revision` of the source entry fails, the new entry
  is still durable. The failure is logged; the next reopen falls
  back to the flat-PNG path if the sidecar is corrupted.
- If `update_metadata` on the source entry fails, the new entry
  is still durable. The failure is logged; the source entry's
  metadata is unchanged.

### 5. Safe fallback for unsupported metadata

`Cache::read_revision` returns `None` when:

- The file is missing.
- The JSON is unparseable.
- The schema version is unsupported.

The IPC layer converts `None` to a `RevisionContext` with
`loader_status: FlatFallback` and an empty annotation list. The
frontend opens the editor with the flattened PNG as the canvas
and no annotations, allowing the user to add new ones — the
acceptance criterion "Unsupported or missing metadata degrades
safely to flattened-image editing".

### 6. Session state machine

Two new states are added to `SessionState`:

- `Reopening` — the source entry is locked and the editor is
  active. Transitions: `Idle -> Reopening` (via `open_revision`),
  `Reopening -> RevisionCommitting` (via `commit_revision`),
  `Reopening -> Idle` (via `cancel_revision`).
- `RevisionCommitting` — the commit pipeline is in flight.
  Transitions: `RevisionCommitting -> Cleanup -> Idle` (on
  commit success), `RevisionCommitting -> Idle` (on commit failure).

The session is the source of truth for "is an editor open?".
A second capture request is rejected with `InvalidSessionState`
when the session is in `Reopening` or `RevisionCommitting`,
matching the existing overlap guard.

## Alternatives considered

- **Pack the editor scene into `metadata.json`**: rejected. The
  user-facing metadata schema and the editor-scene schema evolve
  at different cadences; conflating them couples unrelated changes.
- **Use a separate "editor session" struct on the session
  orchestrator**: rejected. The active-lock registry is the
  source of truth for "is an editor open?", and the `Editor`
  lock is already plumbed through the cache. Adding a new
  session field would duplicate the lock invariant.
- **Rewrite the source entry's PNG in place during commit**:
  rejected. `write_atomic` refuses to overwrite with different
  bytes, and a partial in-place rewrite would leave the entry
  inconsistent between the asset and the manifest. Reusing
  `Cache::commit` for the new entry is the safer path.
- **Ship the original capture's `capture_id` as a parent id on
  the new entry**: rejected. The new entry's `capture_id` is
  already a fresh UUID; the source's shelf id is recorded in
  the new entry's `revision.json`'s `source_shelf_id` field for
  analytics. A formal parent link can land in a future tracer
  if the analytics roadmap demands it.

## Consequences

- The cache layer's `editor_guards` map mirrors `shelf_guards`.
  Any future lock owner follows the same pattern.
- `Cache::commit` now writes a `revision.json` file by default.
  Pre-tracer-10 entries (committed before this branch landed)
  lack the sidecar; the loader treats a missing sidecar as a
  flat-PNG fallback, so the rollout is backward-compatible.
- The session state machine has two new states. Existing tests
  for the capture flow must be updated to handle the new edges
  (handled by the existing `cancel_session` walk-back).
- The frontend annotation store needs a new `loadFromRevision`
  method that wholesale-assigns the runes proxy. The Svelte 5
  reactivity rebuilds from the new references cleanly.
- Every `revision.json` write is a `delete + write_atomic` pair,
  not a true atomic rename. A crash window between the two
  operations leaves a missing file, which the loader treats as
  a flat fallback. This is the correct trade-off: the
  in-progress scene is recoverable from the user's next edit,
  not from a stale persisted file.
- The new IPC commands (`open_revision`, `update_revision`,
  `commit_revision`, `cancel_revision`) follow the contract pair
  pattern: Rust struct + TS mirror + paired tests on both sides.

## Rollout

The IPC commands are registered in `tauri::generate_handler!`. The
cache's `editor_guards` map is initialized as an empty BTreeMap.
The frontend `loadFromRevision` method is backwards-compatible:
existing `OverlayApp.svelte` consumers see no behavior change.
