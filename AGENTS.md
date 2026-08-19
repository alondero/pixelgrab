# AGENTS.md

This file is the primary context for AI implementation agents working on
PixelGrab. It defines the product, the architecture, the domain vocabulary,
the required commands, the testing seams, the privacy rules, and the
expectations for ADR updates. A fresh implementation agent should be able to
identify the architecture, the controlling ADRs, the test seams, and the
privacy constraints from this file and the linked documents alone.

> Maintainers: keep this file current. Every architectural change that
> reaches `main` should be reflected here. ADRs are the canonical authority;
> this file is the navigation aid.

## 1. Product purpose

PixelGrab is a local-first Windows desktop capture and annotation utility. It
makes capture, annotation, temporary storage, and delivery feel like one
continuous desktop gesture. The product is built around a single resident
process that exposes a tray icon, a global region-capture shortcut, a
freeze-frame overlay, and Konva-based annotation tools. Captures are saved
to a temporary cache and offered through a floating shelf with copy, save,
drag-out, and pin-as-reference actions.

The v1 product is described in detail in issue #12. This tracer establishes
the foundation. Subsequent tracers deliver capabilities incrementally.

## 2. Domain glossary

The domain vocabulary is defined in [`docs/GLOSSARY.md`](docs/GLOSSARY.md).
The most important terms every agent must internalize:

- **Capture session** — the lifecycle from idle through capture, overlay,
  selection, editing, commit, and cleanup. See
  [`crates/pixelgrab-contracts/src/session.rs`](crates/pixelgrab-contracts/src/session.rs).
- **Physical coordinate** — a position in actual desktop pixels. ADR-0003
  establishes which layer owns which coordinate system.
- **Virtual desktop** — the union of every monitor's framebuffer, including
  negative origins when monitors are positioned left of or above the primary.
- **Overlay** — the borderless TopMost window that shows the freeze frame
  after capture. Pre-allocated and hidden during setup.
- **Shelf** — the floating queue of recent captures that floats over the
  taskbar. Each card exposes copy, save, pin, and dismiss.
- **Frozen frame** — the immutable RGBA buffer captured before the overlay
  is shown. All annotations are layered on top of this immutable source.
- **Synthetic capture** — a deterministic test framebuffer with no real
  desktop content. The only capture path allowed in CI.
- **Platform contract** — the Rust trait `PixelGrabPlatform` that hides
  Windows-specific capture, monitor, and I/O behind a substitutable boundary.
  See ADR-0002.

## 3. Architecture

```
+--------------------------------------------------+
|                  Tauri shell                     |
|  +---------------+   +-----------------------+   |
|  |    Tray       |   |   Pre-allocated       |   |
|  |  (resident)   |   |   overlay window      |   |
|  +---------------+   +-----------------------+   |
|          |                      |                 |
|          v                      v                 |
|  +------------------ IPC commands ----------------+
|          |                      |                 |
|          v                      v                 |
|  +---------------+   +-----------------------+   |
|  |   Session     |   |  PixelGrabPlatform    |   |
|  | orchestrator  |<->|  (trait, Windows or   |   |
|  |  (state mch)  |   |   synthetic impl)     |   |
|  +---------------+   +-----------------------+   |
|          |                      |                 |
|          v                      v                 |
|  +------------------------------------------+    |
|  |       pixelgrab-contracts (shared)       |    |
|  +------------------------------------------+    |
|          |                                       |
|          v                                       |
|  +------------------ WebView -------------------+|
|  |  Svelte 5 (runes) + Konva.js                ||
|  |  - main window (tray companion)             ||
|  |  - overlay (frozen frame + selection)      ||
|  +---------------------------------------------+|
+--------------------------------------------------+
```

The architecture is documented in detail in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). The data flow for the
synthetic capture end-to-end trace is documented in
[`docs/adr/0001-tauri-svelte-konva-stack.md`](docs/adr/0001-tauri-svelte-konva-stack.md).

## 4. Platform boundaries

- **Windows-specific code** lives behind `src-tauri/src/platform/`. The
  tracer-01 build uses the synthetic implementation exclusively. The
  Windows implementation will be introduced in tracer-02.
- **Platform-neutral code** lives in `crates/pixelgrab-contracts/`. This
  crate must compile on every supported platform.
- **Frontend code** lives in `src/`. It talks to the Rust core only through
  the typed IPC surface in `src/lib/ipc/`.
- **Test adapters** live in `crates/pixelgrab-test-support/`. No test may
  capture real desktop content.

## 5. Capture-session lifecycle

The lifecycle has six states: `idle`, `capturing`, `ready`, `selecting`,
`committing`, `cleanup`. Transitions are validated by the
`SessionOrchestrator::request_transition` method. The allowed edges are
defined in `SessionState::allowed_next()`. Out-of-order transitions are
rejected with `InvalidSessionState`.

The lifecycle is exercised end-to-end by the test in
`src-tauri/tests/session_lifecycle.rs` and the TypeScript test in
`src/lib/ipc/shell.test.ts`.

## 6. Commands

All commands run from the repository root.

| Purpose                                | Command                                                                |
| -------------------------------------- | ---------------------------------------------------------------------- |
| Install dependencies (frozen)          | `pnpm install --frozen-lockfile`                                       |
| Develop the frontend (Vite dev server) | `pnpm dev`                                                             |
| Develop the Tauri shell                | `pnpm tauri:dev`                                                       |
| Build the production binary            | `pnpm tauri:build`                                                     |
| Run the frontend unit tests            | `pnpm test`                                                            |
| Type-check and Svelte-check            | `pnpm check`                                                           |
| Lint (ESLint + Prettier check)         | `pnpm lint`                                                            |
| Format all source files                | `pnpm format`                                                          |
| Run the Rust tests                     | `cargo test --workspace`                                               |
| Run the Rust linter                    | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Format Rust                            | `cargo fmt --all`                                                      |
| License-policy check                   | `pnpm licenses:check`                                                  |
| Regenerate the placeholder icons       | `node scripts/generate-icons.mjs && node scripts/generate-ico.mjs`     |
| Run every quality gate                 | `pnpm ci:all`                                                          |

The Rust and frontend gates are independent and may run in parallel.
The CI pipeline runs them in this order: install -> static -> tests -> build.

## 7. Coding conventions

### Rust

- Apply `cargo fmt` before every commit.
- Clippy must pass with `-D warnings` on every commit.
- Adhere to the rules in the linked `rust-toolchain.toml` (stable).
- Use `thiserror` for structured errors and `PlatformError` for the IPC
  boundary.
- Public APIs must have `///` docs.
- `unsafe` is forbidden except in platform-specific modules and must be
  commented with a `// SAFETY:` block.

### TypeScript

- Use Svelte 5 runes (`$state`, `$derived`, `$effect`) — not the legacy
  `$:` syntax.
- Strict mode is on. No `any`, no implicit `any`.
- All IPC payloads must round-trip against the Rust contract tests.
- ESLint and Prettier must pass on every commit.

### Components

- Components live in `src/lib/<feature>/<Component>.svelte`.
- Cross-component state uses `*.svelte.ts` modules with runes.
- Tests live next to the component (`Component.test.ts`).

## 8. Testing seams

PixelGrab has five testing seams. Each is documented by an example test.

| Seam                     | Example                                   | Tool                      |
| ------------------------ | ----------------------------------------- | ------------------------- |
| Rust unit tests          | `src-tauri/src/session/state.rs` (inline) | `cargo test`              |
| Rust integration tests   | `src-tauri/tests/session_lifecycle.rs`    | `cargo test`              |
| Frontend unit tests      | `src/lib/ipc/types.test.ts`               | `vitest`                  |
| Frontend component tests | `src/App.test.ts`                         | `@testing-library/svelte` |
| Golden image capture     | `src-tauri/tests/golden_capture.rs`       | `cargo test`              |
| Packaged-app acceptance  | `tests/e2e/` (introduced in tracer-14)    | `webdriverio`             |

### Deterministic test adapters

Every test that needs a framebuffer, a monitor layout, a clock, or a
filesystem root must use the `pixelgrab-test-support` crate:

- `SyntheticCapture` — deterministic RGBA framebuffer.
- `SyntheticMonitorLayout` — single, dual, dual-negative, and mixed-DPI
  layouts.
- `ControllableClock` — monotonic + wall clock with manual advance.
- `IsolatedFilesystem` — temporary root under `std::env::temp_dir()` that
  is deleted on drop.

CI never runs a test that captures real desktop content.

## 9. Privacy rules

These rules are non-negotiable and are enforced by the test suite.

- Never log captured pixels, annotation text, clipboard content, or paths
  outside the application cache root.
- The IPC error shape never includes raw file paths from outside the cache.
- Settings files are sanitised before logging.
- The synthetic capture path is the only path allowed in CI.
- Failures that include captured content must scrub the content before
  uploading CI artifacts.

## 9a. Accessibility expectations

PixelGrab is usable from the keyboard alone, respects Windows text scaling,
and never uses colour alone for state. The full checklist is in
[`docs/ACCESSIBILITY.md`](docs/ACCESSIBILITY.md). Highlights:

- Every interactive control exposes a visible label or `aria-label`.
- Focus is always visible.
- Selection and tool state use shape or weight in addition to colour.
- Text renders cleanly at 100%, 125%, 150%, and 200% Windows scaling.

## 10. ADR expectations

Architecture decisions are recorded as ADRs in `docs/adr/`. The current
ADRs are:

- 0001 — Tauri 2 + Svelte 5 + Konva stack
- 0002 — Platform contracts
- 0003 — Physical-coordinate ownership
- 0004 — Packaged-app testing strategy
- 0005 — Cache and one-card shelf (tracer-07)

Additions and revisions must follow the template in
[`docs/adr/README.md`](docs/adr/README.md). When a change supersedes a prior
ADR, mark the original as "superseded" with a forward link — never delete.

## 11. First-tracer scope

The current tracer (tracer-01) is the foundation. It must:

- Establish the build system, tooling, and CI baseline.
- Implement the synthetic capture pipeline end-to-end.
- Define the platform contract and session state machine.
- Provide the resident tray and pre-allocated overlay (without a real capture
  flow yet).
- Document every architectural decision.

Subsequent tracers deliver the real capture pipeline, the multi-monitor
overlay, the annotation tools, the shelf, the OLE drag, and the pin
references. See the issues list for the full roadmap.

## 12. Cache + shelf (tracer-07)

Tracer 07 introduces the durable cache entry and the one-card shelf.
The relevant code lives in:

- `crates/pixelgrab-contracts/src/cache.rs` — `CacheEntry`,
  `CacheEntryMetadata`, `ShelfPosition`, `LockOwner`. Mirrored in
  `src/lib/ipc/types.ts`.
- `src-tauri/src/cache/` — `atomic::write_atomic`, `locks::ActiveLockSet`,
  `store::Cache`. The two-phase commit lives in `Cache::commit`.
- `src-tauri/src/shelf/mod.rs` — the one borderless webview window
  (`label = "shelf"`). Position calculation lives in
  `pixelgrab_contracts::ShelfPosition::inside_primary_work_area`.

### Two-phase commit

Every capture commit writes assets first (PNG, optional bitmap,
metadata), then the manifest. The manifest is the publish sentinel:
the shelf only enumerates entries with a `manifest.json`. A crashed
commit (assets present, no manifest) is reaped by
`Cache::load_or_recover` on the next startup scan.

### Active locks

Every committed entry holds a `Shelf` lock for the duration of its
card. Other consumers (editor, drag, pin) acquire additional locks as
needed. Cleanup (`Cache::dismiss`) is rejected while any owner holds
the lock; see `LockOwner` for the exhaustive list.

### IPC surface

New IPC commands (registered in `src-tauri/src/lib.rs`):

- `request_commit` — runs the two-phase commit pipeline
  (`flatten_crop` → optional clipboard → optional cache commit →
  optional `save_as` PNG). The IPC layer publishes the clipboard
  _before_ committing to the cache so a clipboard error never leaves
  a phantom card. `session.finish()` runs once at the end of every
  commit attempt so the session is always reset to `Idle` — even on
  failure.
- `update_cache_metadata` — atomic rewrite of `metadata.json` and
  refresh of `manifest.json`'s `lastAccessAtMs`.
- `dismiss_cache_entry` — releases the `Shelf` lock and reaps the
  entry when no other locks remain. Emits
  `pixelgrab://shelf-cleared` with a typed `{ shelfId }` payload so
  the frontend listener can match its parameter.
- `get_shelf_snapshot` — returns the current `ShelfSnapshot` for
  frontend rehydration after a process restart.

Events:

- `pixelgrab://shelf-updated` carries a `ShelfCardView` serialised
  from the latest `CacheEntry`. The shelf window subscribes to this
  event in `src/shelf.ts`.
- `pixelgrab://shelf-cleared` carries a typed
  `ShelfClearedEvent { shelfId: string }` payload. Emitted when a
  dismissal removes the entry from disk.

## 13. Shelf queue, timers, and quick actions (tracer-08)

Tracer 08 generalises the one-card shelf into a queue of up to four
visible cards with an expandable `+N` overflow group. Per-card timers
pause on hover and resume with a three-second grace period; quick
actions (Copy, Save As, Dismiss) operate on the targeted card. The
relevant code lives in:

- `crates/pixelgrab-contracts/src/shelf_queue.rs` — `ShelfTimerConfig`,
  `ShelfTimerState`, `ShelfQueueCard`, `ShelfQueueSnapshot`,
  `CopyShelfCardRequest`, `SaveShelfCardAsRequest`. Mirrored in
  `src/lib/ipc/types.ts`.
- `src-tauri/src/shelf/queue.rs` — `ShelfQueueEngine`. Owns the
  ordered list and per-card timer state; mirrors the cache but never
  duplicates its lock or persistence invariants.
- `src/lib/shelf/ShelfQueue.svelte` and `ShelfCard.svelte` — the
  Svelte components that render the multi-card row + overflow panel.
- `src/lib/shelf/queue.svelte.ts` — per-card countdown state driven
  by `requestAnimationFrame` so the visual countdown updates without
  a backend round-trip.

### Queue engine ↔ cache

The queue engine mirrors the cache; the cache still owns durability
and the shelf lock. Every IPC layer mutation calls both: commit →
`cache.commit` then `queue.add`; dismiss → `cache.dismiss` then
`queue.dismiss`; tick → `queue.tick` returns expired ids and the IPC
layer calls `cache.dismiss` on each. The cache lock is therefore
released on every terminal path (commit, manual dismiss, expiry in
either main view or overflow) without duplicating the lock invariant.

### Per-card timers

Timers use monotonic elapsed millis (driven by `performance.now()` on
the frontend, `SystemTime` on the backend). On hover the frontend
calls `hover_shelf_card`; the backend captures the remaining time.
On leave the backend re-establishes the deadline as
`now + max(paused_remaining, grace_ms)` so a card with very little
remaining time still gets a fair chance to be read.

### IPC surface

New IPC commands (registered in `src-tauri/src/lib.rs`):

- `copy_shelf_card` — reads the cached PNG and publishes it to the
  system clipboard via `PixelGrabPlatform::publish_png_clipboard`.
- `save_shelf_card_as` — opens the native Save As dialog and writes
  the cached PNG bytes to the chosen path.
- `hover_shelf_card` / `unhover_shelf_card` — pause or resume the
  targeted card's timer.
- `tick_shelf_queue` — run one expiry pass on the queue; the backend
  dismisses each expired id from the cache.
- `get_shelf_queue_snapshot` — return the current `ShelfQueueSnapshot`
  for frontend rehydration.

Events:

- `pixelgrab://shelf-queue-updated` carries a `ShelfQueueSnapshot`
  with all visible cards + overflow, their per-card timers, and the
  computed window position. The frontend re-renders the queue from
  the payload alone.

## 14. External drag pipeline (tracer-09)

The external drag lives behind the `PixelGrabPlatform::start_drag`
trait method (`crates/pixelgrab-contracts/src/drag.rs`). The platform
contract hides the Windows OLE pipeline behind the trait; the
synthetic adapter is the fault-injection seam for CI.

The drag offers four clipboard formats from a single stable PNG:
`CF_HDROP`, a registered PNG format, `CF_DIBV5`, and
`CF_UNICODETEXT`. The `OleState` owns the PNG bytes for the full
synchronous `DoDragDrop` call, so the cache layer must hold a `Drag`
lock on the entry for the duration of the call.

The terminal outcome is one of `Accepted`, `Rejected`, `Cancelled`,
or `Failed`. Only `Accepted` triggers the optional card dismissal.
The `DragDiagnostics` record carries the formats the target pulled,
the timings, the target effect, and the categorical failure kind —
never the captured pixels or the absolute PNG path.

The Windows adapter is hand-rolled on a minimal COM vtable so the
build does not depend on the `windows` crate's macro evolution.
Tests are wired into `synthetic_capture`, `session_lifecycle`, and
the IPC contract suites.
