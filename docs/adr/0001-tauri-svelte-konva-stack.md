# ADR-0001: Tauri 2 + Svelte 5 + Konva stack

## Status

Accepted (tracer-01).

## Context

PixelGrab is a Windows desktop capture utility with a vector annotation
canvas and a resident tray UI. We needed to choose a process shell, a UI
framework, and a canvas engine that:

- Ship a single self-contained binary for Windows.
- Can run as a resident tray application with a global hotkey.
- Provide a WebView that can render a vector canvas fluently.
- Have a strong ecosystem of plugins (single-instance, global-shortcut,
  dialog, clipboard, fs).
- Remain testable without a real desktop.

## Decision

We adopt:

- **Tauri 2** as the process shell. It produces small Windows binaries,
  integrates with the system tray via `tauri-plugin-tray`, registers
  global shortcuts via `tauri-plugin-global-shortcut`, enforces the
  single-instance invariant via `tauri-plugin-single-instance`, and
  exposes a typed IPC that we control.
- **Svelte 5 (with runes)** as the UI framework. The new rune syntax
  (`$state`, `$derived`, `$effect`) is explicit, plays well with
  TypeScript, and integrates with the existing Vite + Testing Library
  ecosystem.
- **Konva.js** as the canvas engine. Konva provides a typed scene graph
  with transformers, an event system, and image caching. It avoids the
  complexity of writing a custom 2D canvas abstraction while remaining
  lighter than `fabric.js`.
- **Vite** as the build tool. Vite is the recommended bundler for the
  SvelteKit ecosystem and integrates with Tauri's dev server.
- **pnpm** as the package manager. pnpm is faster than npm, has strict
  peer-dependency resolution, and is the recommended package manager for
  Tauri + monorepo projects.
- **TypeScript 5** for the frontend. Provides static guarantees for the
  IPC payload types.

## Consequences

### Positive

- A single Tauri binary with a WebView contains the entire UI.
- The Svelte 5 + runes model is explicit and easy to test.
- Konva's scene graph maps directly to the annotation model.
- pnpm workspaces and the Rust workspace model align cleanly.

### Negative

- The WebView is a separate process; we must use a typed IPC.
- Konva's image rendering can be slower than a raw canvas for high
  pixel-count operations (e.g. blur). We mitigate this by composing
  the blur into the final commit bitmap rather than animating it.
- Svelte 5 runes are still a relatively new API; some plugins lag.

### Trade-offs

- We accept the WebView integration cost in exchange for a fast and
  familiar UI framework.
- We accept Konva's 9.x API surface in exchange for a complete
  vector scene graph.

## Alternatives

- **Electron.** Larger binaries, more memory, and a less principled
  tray-icon story. Rejected.
- **Wails.** The Tauri equivalent for Go. Rejected because the team
  standardises on Rust for native code.
- **React + react-konva.** Rejected because Svelte 5's reactivity
  model is a better fit for the overlay's interactions.
- **Custom canvas code.** Rejected; the canvas code would grow
  significantly for the same feature set.
