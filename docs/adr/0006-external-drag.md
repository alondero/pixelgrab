# ADR-0006: External drag-and-drop pipeline

## Status

Accepted · 2026-08-19 · Tracer 09

## Context

A shelf card must travel to external Windows applications — Chromium
browsers, Electron apps, Windows Explorer, and IDEs — when the user
drags the thumbnail off the shelf. The native drag-and-drop protocol
on Windows is COM **OLE** (`IDataObject` + `IDropSource` +
`DoDragDrop`), and the requirements are:

- Multiple clipboard formats must be offered from one stable capture.
- The drag must be **retryable** if the user cancels, the target
  rejects, or the OLE pipeline itself fails.
- Backing assets must never be pruned while OLE may request them.
- The diagnostics surface must record what the target requested, the
  timings, the target effect, and the error — but never the captured
  pixels.
- The contract must be portable so a future macOS implementation can
  sit beside the Windows adapter.

## Decision

The external drag pipeline is delivered as a new `PixelGrabPlatform`
trait method, `start_drag`, surrounded by a separate `drag` module
in `pixelgrab-contracts` that owns the wire shapes. The implementation
split:

1. **Platform-neutral contract** (`pixelgrab-contracts::drag`)
   - `DragRequest`, `DragResult`, `DragDiagnostics`, `DragOutcome`,
     `DragFormat`, `DragTargetEffect`, `DragTargetKind`.
   - Every type is `serde`/`serde_json` compatible so the IPC layer
     can project the result without translation.
2. **Synthetic adapter** (`platform::drag_synthetic`)
   - Deterministic replacement for the COM pipeline so CI can drive
     the four terminal outcomes without a desktop session.
   - Scriptable failure injection (`SyntheticDragScript::Cycle` and
     `::AlwaysFail`) for property tests.
   - Tracks file handles against the drag loop so the leak guard
     asserts `held_paths.is_empty()` after every run.
3. **Windows adapter** (`platform::windows::drag`)
   - Hand-rolled COM vtables for `IDataObject`, `IDropSource`, and
     `IEnumFORMATETC`. The `windows` crate's macro-driven COM stack
     has churned between versions; the hand-rolled surface is small
     (~400 lines) and stable.
   - Four clipboard formats: `CF_HDROP`, a registered PNG format
     (`RegisterClipboardFormatW("image/png")`), `CF_DIBV5`, and
     `CF_UNICODETEXT`.
   - Hand-rolled `BITMAPV5HEADER` packing for `CF_DIBV5` so the
     `bgra_pixels` buffer is the single source of truth.
   - `DoDragDrop` is the only `ole32` symbol imported; the hand-rolled
     vtables replace the `windows` crate's macro output.
4. **IPC layer** (`start_shelf_drag`)
   - The Rust side hands the platform contract a `DragRequest` and
     returns a `StartShelfDragResult` with the terminal outcome and
     the dismiss hint.
   - The dismiss hint is `true` only when the configured policy is
     `dismiss_on_accepted` and the outcome is `Accepted`. Rejected,
     cancelled, and failed drags retain the card.

## Drag outcomes and HRESULT mapping

| Outcome     | Source                                                         | Card retained? |
| ----------- | -------------------------------------------------------------- | -------------- |
| `Accepted`  | `DoDragDrop` returned `DRAGDROP_S_DROP` with `DROPEFFECT_COPY` | No (dismiss)   |
| `Rejected`  | Target returned `DROPEFFECT_NONE` or zero effect               | Yes            |
| `Cancelled` | User pressed Escape or released outside a target               | Yes            |
| `Failed`    | Negative HRESULT from `DoDragDrop` or OOM on allocation        | Yes            |

The HRESULT-to-outcome translation is the single source of truth
(`platform::windows::drag::translate_hr`). Tests assert the
mapping for every sentinel.

## File handling

The PNG bytes are read into memory once at `start_drag` entry and
held in the `OleState` for the full synchronous `DoDragDrop` call.
The Rust contract with the shelf/cache layer is:

> The `png_path` field is the absolute path of a stable PNG. The
> file must not be pruned, renamed, or rewritten for the duration of
> the `DragResult` return.

The implementation guarantees this by reading the file once and
holding the bytes through the lifetime of the `OleState` (which is
tied to the `IDataObject` reference count). When `DoDragDrop`
returns, the `IDataObject` is dropped, the `OleState` is freed, and
the file lock is released.

## Diagnostics

`DragDiagnostics` carries:

- `started_at_ms`, `completed_at_ms`, `duration_ms` — wall clock.
- `requested_formats: Vec<DragFormatRequest>` — the formats the drop
  target actually pulled during the drag loop. Populated from
  `IDataObject::GetData` calls.
- `target_effect: DragTargetEffect` — one of `copy`, `move`, `none`,
  `unknown`.
- `target_kind: DragTargetKind` — categorical drop-target class.
  The Windows adapter does **not** introspect the target window
  (process IDs / window handles would leak into diagnostics). The
  default is `Other`; a future tracer may wire process introspection
  when contract-acceptable.
- `failure_kind: Option<String>` — the categorical `PlatformErrorKind`
  label, only present when the outcome is `Failed`.

The struct never includes the PNG path, the BGRA pixels, the
absolute filesystem path, or process IDs.

## Consequences

- The platform contract has one new method (`start_drag`). The
  default implementation rejects the call with `Unsupported` so the
  synthetic adapter has to opt in explicitly.
- The COM surface is hand-rolled. The trade-off is more code, but
  the build does not depend on the `windows` crate's macro
  evolution.
- The diagnostics record is privacy-safe; the IPC layer can pass it
  to telemetry without scrubbing.
- The cache layer (tracer-07) must hold a `Drag` lock on the cache
  entry for the full synchronous call. The Rust contract forces the
  cache to look up the PNG bytes from the entry's stored path before
  the IPC transition.

## Alternatives

- **`windows` crate with `#[implement]` macro.** Rejected because
  the macro API has changed between 0.55 and 0.58 and the required
  `windows_core` direct dependency is gated on a specific feature
  combination. Hand-rolled vtables are stable.
- **A separate crate for the drag logic.** Rejected: the drag is
  tightly coupled to the platform contract and the IPC layer.
- **Async drag via `tokio::task::spawn`.** Rejected: `DoDragDrop`
  runs a modal loop on the calling thread. Trying to wrap it in a
  future deadlocks because the modal loop pumps window messages.
- **A single `PNG` format only.** Rejected: Chromium accepts the
  registered PNG format but Explorer prefers `CF_HDROP`; legacy
  bitmap targets need `CF_DIBV5`. The four-format set is the
  customary "good citizen" minimum.

## Future work

- Process introspection for the `target_kind` discriminator when
  the privacy contract is updated.
- A `DropEffect::Move` mapping (currently treated as `Copy` so the
  shelf PNG is never deleted unilaterally).
- A `pixelgrab://shelf-card-dragged` event so the frontend can
  update per-card state without polling.
