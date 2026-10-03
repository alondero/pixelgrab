# Architecture overview

This document describes the PixelGrab architecture. For the controlling
architectural decisions see the ADRs in [`docs/adr/`](adr/). For the
context that informs this architecture see issue #12.

## Goals

PixelGrab's architecture is built around five goals:

1. **Local-first.** No network egress; all capture, annotation, and storage
   happens on the user's machine.
2. **Single resident process.** One app, one tray, one overlay. Secondary
   launches forward intents to the running primary.
3. **Platform-neutral core.** The capture-session, annotation, and shelf
   behaviour are platform-neutral; Windows-specific code is hidden behind
   platform contracts.
4. **Deterministic testability.** Every seam has a synthetic implementation
   that CI can drive without touching real desktop content.
5. **Privacy-respecting.** No pixel, annotation, or path leaves the
   cache root in diagnostics.

## Process model

```
                  +-----------------------+
                  |  Primary PixelGrab    |
                  |  process (resident)   |
                  +-----------+-----------+
                              |
        +---------------------+---------------------+---------------------+
        |                     |                     |                     |
        v                     v                     v                     v
   Tray icon           Overlay window       single-instance       cache + settings
   (visible)           (hidden until        plugin (rejects            worker
                       capture)             second primary)
```

A secondary launch is caught by `tauri-plugin-single-instance`. Tray, hotkey,
and secondary-launch capture intents share the native dispatcher in
`singleton::forward_to_existing_instance`; capture never depends on a hidden
WebView relaying an event. Settings presentation still uses the UI event bus.

## Modules

### Rust

| Module                      | Purpose                                                         |
| --------------------------- | --------------------------------------------------------------- |
| `pixelgrab_lib::error`      | Internal error types. Wraps the contract error.                 |
| `pixelgrab_lib::session`    | The capture-session state machine.                              |
| `pixelgrab_lib::platform`   | The `PixelGrabPlatform` trait and the synthetic implementation. |
| `pixelgrab_lib::ipc`        | The Tauri command handlers.                                     |
| `pixelgrab_lib::tray`       | The resident tray icon and menu.                                |
| `pixelgrab_lib::overlay`    | The pre-allocated overlay window.                               |
| `pixelgrab_lib::singleton`  | Single-instance intent forwarding.                              |
| `pixelgrab_contracts::*`    | Platform-neutral types and the IPC contract.                    |
| `pixelgrab_test_support::*` | Deterministic test adapters.                                    |

### TypeScript

| Module                                   | Purpose                                          |
| ---------------------------------------- | ------------------------------------------------ |
| `src/App.svelte`                         | Recent screenshots, editor, and settings.        |
| `src/lib/shelf/controller.svelte.ts`     | Shared recent-capture subscriptions and actions. |
| `src/lib/revision/RevisionEditor.svelte` | Immutable-source visual revision editor.         |
| `src/lib/overlay/OverlayApp.svelte`      | The overlay window.                              |
| `src/lib/overlay/KonvaStage.svelte`      | The frozen-frame Konva stage.                    |
| `src/lib/stores/session.svelte.ts`       | The session state rune store.                    |
| `src/lib/ipc/commands.ts`                | Tauri command wrappers.                          |
| `src/lib/ipc/shell.svelte.ts`            | A Tauri-free mock used by tests.                 |
| `src/lib/ipc/types.ts`                   | The IPC payload types (mirrors Rust).            |

## Capture, sharing, and revision flow

1. Tray/hotkeys/secondary launch dispatch capture natively. Main-window buttons
   reach the same owner through IPC.
2. `capture_native` hides PixelGrab's companion and shelf, freezes pixels, and
   records the capture. Region capture reveals the preallocated overlay and
   pushes `capture-ready`; full-screen capture commits the cursor's monitor
   immediately without opening the overlay.
3. Konva maps region gestures from WebView coordinates to physical desktop
   bounds. Release commits by default. Ctrl/Meta or Edit before sharing keeps
   the crop open for annotation, where Enter/Copy/Done commit explicitly.
4. The shared flatten pipeline produces the export and clipboard bitmap.
   `commit_editable` publishes immutable `source.png`, flattened `capture.png`,
   scene, and metadata before the manifest, then registers the shelf entry.
5. Native shelf and main gallery subscribe before fetching their startup
   snapshot. Both show recent images and preserve newer events over delayed
   startup reads. Placement scales logical card sizes into physical bounds and
   moves excess visible cards into overflow.
6. Drag holds a cache guard before reading the exported PNG, pauses the queue
   timer, and offers file/bitmap formats through the platform. Success retains
   the Shelf guard so targets can read later and the screenshot can be reused.
7. Revision open acquires the Editor guard and owns native reveal/event delivery.
   The visual editor restores crop-local vectors over the immutable source.
   A revision commit publishes a new entry without changing the original;
   failure restores a retryable editor session. Missing/unsupported scene or
   source falls back to the flattened image and an empty vector scene.
8. Capture terminal paths release the session and hide the overlay. Native
   companion windows survive close requests; closing an editor cancels its
   session. See [ADR-0012](adr/0012-quick-capture-and-editable-sources.md).

## Security

- The CSP restricts the WebView to the local origin and the asset protocol.
- All IPC payloads are JSON. No code is deserialised from the WebView.
- The Rust side never trusts a payload; payloads are validated against the
  contract types before being applied.
- The shader and image data paths are isolated from the wider filesystem.

## Testing

| Layer                   | Tool                                          |
| ----------------------- | --------------------------------------------- |
| Rust unit/integration   | `cargo test --workspace --features synthetic` |
| Frontend unit           | `vitest`                                      |
| Frontend component      | `@testing-library/svelte`                     |
| Golden image            | `cargo test` (with `png` decoder)             |
| Packaged-app acceptance | `webdriverio` (introduced in tracer-14)       |

## Future extensions

- **macOS** — the platform contract already hides the Windows-specific
  code. A macOS implementation only needs to satisfy `PixelGrabPlatform`.
- **Linux** — out of scope per issue #12.
- **Cloud upload** — out of scope. Local-first only.
- **OCR** — out of scope.
