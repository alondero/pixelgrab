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

A secondary launch is caught by `tauri-plugin-single-instance` and emits an
intent to the running primary via the existing event bus. The primary then
brings the overlay to the foreground (or shows the main window for the
configuration links).

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

| Module                              | Purpose                               |
| ----------------------------------- | ------------------------------------- |
| `src/App.svelte`                    | The main window (tray companion).     |
| `src/lib/overlay/OverlayApp.svelte` | The overlay window.                   |
| `src/lib/overlay/KonvaStage.svelte` | The frozen-frame Konva stage.         |
| `src/lib/stores/session.svelte.ts`  | The session state rune store.         |
| `src/lib/ipc/commands.ts`           | Tauri command wrappers.               |
| `src/lib/ipc/shell.svelte.ts`       | A Tauri-free mock used by tests.      |
| `src/lib/ipc/types.ts`              | The IPC payload types (mirrors Rust). |

## Data flow — synthetic capture

The synthetic end-to-end flow exercised by tracer-01:

1. The user clicks **Capture Region** in the tray.
2. The tray emits `pixelgrab://request-capture` to the main window.
3. The frontend calls `requestCapture` (Tauri IPC).
4. The Rust `request_capture` handler invokes
   `SessionOrchestrator::run_capture`.
5. The orchestrator transitions to `Capturing`, calls the platform
   contract, and writes the resolution to internal state.
6. The frontend receives the `CaptureResolution` (which includes a
   `data:` URL for the synthetic PNG).
7. The frontend forwards the URL to the overlay window.
8. The overlay renders the freeze frame with Konva.
9. The user drags a region. The overlay emits a `RequestOverlayIntent`
   with the physical bounds.
10. The user presses Enter. The frontend calls `requestCommit`.
11. The Rust `request_commit` handler writes the flattened PNG to disk
    and returns a `CommitOutcome`.
12. The frontend updates the session state to `idle`.

## Security

- The CSP restricts the WebView to the local origin and the asset protocol.
- All IPC payloads are JSON. No code is deserialised from the WebView.
- The Rust side never trusts a payload; payloads are validated against the
  contract types before being applied.
- The shader and image data paths are isolated from the wider filesystem.

## Testing

| Layer                   | Tool                                    |
| ----------------------- | --------------------------------------- |
| Rust unit               | `cargo test`                            |
| Rust integration        | `cargo test --workspace`                |
| Frontend unit           | `vitest`                                |
| Frontend component      | `@testing-library/svelte`               |
| Golden image            | `cargo test` (with `png` decoder)       |
| Packaged-app acceptance | `webdriverio` (introduced in tracer-14) |

## Future extensions

- **macOS** — the platform contract already hides the Windows-specific
  code. A macOS implementation only needs to satisfy `PixelGrabPlatform`.
- **Linux** — out of scope per issue #12.
- **Cloud upload** — out of scope. Local-first only.
- **OCR** — out of scope.
