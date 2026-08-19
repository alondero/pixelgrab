# Glossary

This document defines the domain vocabulary used throughout PixelGrab. The
terms are also referenced from `AGENTS.md`. When introducing new terms,
add them here.

## Capture

- **Capture** — the act of reading pixels from the desktop framebuffer.
  Implemented behind the `PixelGrabPlatform::capture` contract.
- **Capture session** — the lifecycle from idle through capture, overlay,
  selection, editing, commit, and cleanup.
- **Capture id** — a UUID v4 assigned by the Rust core. Stable for the
  lifetime of the cache entry.
- **Frozen frame** — the immutable RGBA buffer captured before the overlay
  is shown. All annotations are layered on top of this immutable source.
- **Synthetic capture** — a deterministic test framebuffer with no real
  desktop content. The only capture path allowed in CI.

## Coordinate systems

- **Client coordinates** — coordinates relative to the WebView's CSS
  origin. Used by Konva.
- **Physical coordinates** — positions in actual desktop pixels. Always
  non-negative. The canonical wire format.
- **Virtual desktop coordinates** — the union of every monitor's
  framebuffer, including negative origins when monitors are positioned
  left of or above the primary. See `VirtualBounds`.
- **Capture buffer coordinates** — physical coordinates relative to the
  captured framebuffer's origin (top-left of the virtual desktop).
- **Export crop coordinates** — the physical coordinates of the final
  flattened PNG.

## Monitor

- **Monitor** — a single display. Identified by a stable `id` that
  survives topology changes for the same physical device.
- **Monitor layout** — the ordered list of currently-connected monitors.
- **Primary monitor** — the monitor designated by the OS as primary.
- **Work area** — the area of a monitor that is not consumed by the
  taskbar or other docked toolbars.

## Overlay

- **Overlay** — the borderless TopMost window that shows the freeze frame
  after capture. Pre-allocated and hidden during setup.
- **Pre-allocated overlay** — the overlay window is created during app
  setup and hidden. The first capture does not pay a window-creation cost.

## Annotation

- **Annotation** — a vector entity drawn on top of the frozen frame.
- **Region-first** — the interaction model: the user selects a region
  first, then annotation tools become active.
- **Annotation tools** — Arrow, Rectangle, Text, Blur, Numbered Badge,
  Select. Each has a keyboard shortcut.
- **Style palette** — Red, Green, Blue, Yellow, White. Plus 2 px, 4 px,
  and 8 px stroke widths.

## Shelf

- **Shelf** — the floating queue of recent captures that floats over
  the taskbar. Tracer 07 ships one card; later tracers stack more.
- **Shelf card** — a single capture on the shelf. Exposes copy, save,
  pin, and dismiss actions.
- **Shelf placement** — the shelf window is always 24 px inside the
  primary monitor's work area, anchored to the bottom-right. The
  calculation lives in
  `pixelgrab_contracts::ShelfPosition::inside_primary_work_area`.
- **Shelf timer** — the 60-second default countdown for each card. Hover
  pauses it; leave resumes with a three-second grace period.
- **`pixelgrab://shelf-cleared`** — event the backend emits when a
  dismissal removes an entry from disk. Carries a typed
  `{ shelfId: string }` payload; the frontend uses it to clear its
  local card.
- **Pin** — a TopMost reference window that displays a captured image.
  Independent of the shelf. Acquires a `LockOwner::Pin` lock on the
  backing cache entry so the entry cannot be reaped while the pin is
  alive.

## Delivery

- **OLE drag** — the native drag-and-drop protocol used to send a
  capture to other applications. Offers CF_HDROP, PNG, CF_DIBV5, and
  CF_UNICODETEXT.
- **Drop-target** — the application receiving a drag. Chromium browsers,
  Electron apps, Windows Explorer, and IDEs are the primary targets.

## Settings

- **Settings file** — the versioned JSON file at
  `%LOCALAPPDATA%\com.pixelgrab.app\settings.json`.
- **Last-known-good** — the previous valid settings file preserved as
  `settings.json.bak`. Used when the primary file is corrupt.
- **Atomic write** — settings are written to a sibling temp file,
  flushed, then renamed to the primary path.

## Cache

- **Cache entry** — a single capture stored in the local cache. Has a
  PNG, an optional bitmap staging asset, optional metadata, a byte
  size, a creation time, and a last-access time. Each entry is a
  directory under `<cache_root>/<capture_id>/` containing
  `capture.png`, optional `bitmap.png`, `metadata.json`, and a final
  `manifest.json` (the publish sentinel).
- **Active lock** — a guard that prevents a cache entry from being
  pruned while it is visible in the shelf or a pin window. Owners are
  typed as `LockOwner::{Shelf, Editor, Drag, Pin}`. The cache always
  holds a `Shelf` lock for the lifetime of the card.
- **Two-phase commit** — the cache writes assets first (PNG, bitmap,
  metadata), then writes the manifest. The manifest is the publish
  sentinel; the shelf only ever sees entries with a manifest.
- **LRU** — least-recently-used; the cache eviction order.
- **Recovery scan** — the startup pass that reaps any partial entry
  (assets present, manifest absent). Runs on every process start.

## Lifecycle

- **Idle** — no overlay. The pre-allocated overlay is hidden.
- **Capturing** — native capture is running against all monitor
  framebuffers.
- **Ready** — frame captured, overlay is about to be shown.
- **Selecting** — user is selecting a region and/or editing annotations.
- **Committing** — commit/cancel cleanup is in progress.
- **Cleanup** — overlay being hidden, locks released.

## Quality

- **CI** — the GitHub Actions pipeline that runs every quality gate.
- **Quality gate** — a check that must pass before a PR can merge.
- **Golden image** — a checked-in reference PNG used for visual
  regression tests.
- **Contract test** — a test that verifies the Rust and TypeScript
  sides of the IPC stay in sync.
- **WebdriverIO** — the end-to-end test runner for the packaged app.

## Privacy

- **Privacy boundary** — the flattened bitmap is the privacy boundary;
  exports must never omit the redaction layer.
- **Scrubbed diagnostics** — diagnostic output that has been stripped of
  pixel data, annotation text, clipboard content, and external paths.
