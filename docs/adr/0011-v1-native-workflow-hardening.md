# ADR-0011: v1 native workflow hardening (issue #63)

## Status

Accepted. Amends ADR-0006 (external drag), ADR-0007 (cache bounds and
recovery), and ADR-0010 (overlay reveal seam) as described below.

## Context

The 2026-08-22 v1 gap review against #12 and #1 left nine release
blockers (issue #63): shelf-card Pin / Reopen / Drag were unwired,
pins had no native windows, pin and drag ownership bypassed the cache
lock registry, work areas were synthetic, display changes were only
handled manually in tests, cursor targeting and the placement preview
were missing, mixed-DPI overlay mapping was unvalidated, the full
freeze frame crossed IPC as base64, and the packaged WebDriver suite
was a title check.

Two production defects found while validating on real hardware shaped
the design:

- A statically declared Tauri window without an explicit `url` loads
  `index.html`; because window preallocation early-returns on an
  existing label, the real overlay entrypoint was never mounted.
- A pre-allocated webview lives across captures, so any
  mount-time-only state read goes stale from the second capture on.

## Decision

1. **Per-pin native windows.** Every pin is its own borderless,
   always-on-top, taskbar-hidden webview window labelled `pin-{pinId}`
   loading `pin.html?id=…`. The entrypoint fetches its view model with
   `get_pin` (no event race); updates arrive as targeted
   `pixelgrab://pin-viewmodel` events. Every close route (IPC, context
   menu, OS close) releases the registry entry.
2. **Shared lock registry with refcounts.** `ActiveLockSet` owners are
   reference-counted: each `acquire` takes a ref, each guard drop
   releases one, and an owner survives until its last ref drops. Pins
   take `LockOwner::Pin` refs through `CachePinLockProvider` (resolved
   `capture_id → shelf_id` via the cache); drags hold an RAII
   `LockOwner::Drag` guard for the whole synchronous OLE loop.
3. **Drag payload assembly moves backend-side.** `StartShelfDragIntent`
   is now `{ shelf_id, dismiss_on_accepted }`; the IPC layer reads the
   cached PNG, decodes the BGRA bitmap, and validates the
   `DragRequest`. This amends ADR-0006: the heavy payload never
   crosses IPC.
4. **Bounded local asset transport.** Freeze frames are encoded once
   and written atomically to `<cache-root>/frames/{capture_id}.png`
   (64 MiB bound); `CaptureResolution.asset_url` carries the file path
   and the webview loads it through the asset protocol. No cache root
   configured → inline data-URL fallback (synthetic/CI). Orphaned
   frames are reaped at startup (all) and by the periodic sweep
   (older than one sweep interval); the debris pass excludes
   `frames/`. This amends ADR-0007's recovery sweep scope.
5. **Capture-ready push contract.** `request_capture` emits
   `pixelgrab://capture-ready` carrying the fresh `CaptureResolution`
   after the reveal seam runs. This amends ADR-0010: the backend seam
   still owns positioning and the `Ready → Selecting` walk, but the
   overlay page is additionally _pushed_ the capture because the
   long-lived webview's mount-time read goes stale.
6. **Display watcher.** A background thread polls the platform layout
   every 3 seconds and compares an order-sensitive Fingerprint of
   every monitor's id/bounds/scale/work-area (Windows offers no
   webview-reachable topology event). On change it invalidates the
   cached layout, re-anchors pins into the work-area union,
   repositions the shelf, and emits `pixelgrab://display-changed`.
7. **Anchored-monitor DPI for the overlay.** A top-level window has
   exactly one DPI context, so the overlay's logical conversion uses
   the anchored (primary) monitor's scale factor resolved from the
   layout — not the WebView's stale `current_monitor()`.
8. **Real work areas and cursor targeting.** Windows work areas come
   from hand-rolled `EnumDisplayMonitors`/`GetMonitorInfoW` FFI
   (no `windows` crate dependency, matching ADR-0006's convention);
   `cursor_position()` on the platform contract resolves the
   `"cursor"` shelf target.

## Consequences

- Easier: pins, drags, and shelf cards answer "is this entry in use?"
  from one registry; the sweeper and manual clear honour all of them.
- Easier: multi-MB freeze frames no longer cross IPC as base64.
- Harder: the lock set's refcount semantics must be understood by
  anyone touching cache eviction (a guard drop releases one ref, not
  the owner).
- Harder: per-pin windows multiply webview processes; the pin ceiling
  (`MAX_PINS`) bounds it.
- Accepted: display changes are noticed within one poll interval
  (≤3 s), not instantly.
- Accepted: `reap_frame_assets` compares wall-clock mtimes, so the
  injected clock in tests must be wall-clock-based.

## Alternatives

- **WM_DISPLAYCHANGE via a hidden message window** — rejected for now;
  substantially more unsafe FFI for latency the shelf does not need.
- **Frontend assembles the drag payload** (status quo) — rejected: it
  forced multi-MB BGRA arrays through IPC and duplicated cache reads.
- **Overlay polls `get_session_snapshot`** — rejected: polling in a
  hidden webview wastes cycles and still lags the reveal.
- **`windows` crate for monitor info** — rejected per the ADR-0006
  convention of hand-rolled FFI to avoid macro-evolution churn.
