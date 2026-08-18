# ADR-0002: Platform contracts

## Status

Accepted (tracer-01).

## Context

PixelGrab needs to support native capabilities on Windows: capture,
monitor layout, file I/O, clipboard, dialog, drag, and pinned windows.
The product spec (#12) requires that a future macOS port can reuse the
capture-session, annotation, and shelf behaviour. Without a deliberate
boundary, Windows-specific code would leak into the orchestration layer
and the macOS port would require a rewrite.

## Decision

We define a `PixelGrabPlatform` trait in
`src-tauri/src/platform/contract.rs` that exposes the platform-dependent
operations the orchestrator needs:

- `monitor_layout(&self) -> PlatformResult<MonitorLayout>`
- `capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution>`
- `write_png(&self, capture_id, bounds, rgba) -> PlatformResult<PathBuf>`

The trait is `Send + Sync` so the orchestrator can be wrapped in an
`Arc` and shared across Tauri command handlers.

The workflow for adding a platform:

1. Add a new module under `src-tauri/src/platform/<os>/` that implements
   `PixelGrabPlatform`.
2. Wire the new module into `PixelGrabApp::new` in `lib.rs`.
3. Add conformance tests that exercise the contract with the synthetic
   adapter.

## Consequences

### Positive

- The orchestrator is platform-neutral and reusable for macOS.
- Tests can drive the orchestrator with the synthetic adapter; no
  Windows-runner is required for unit tests.
- The contract surface is small and easy to audit.

### Negative

- The contract grows over time. Each new capability (e.g. drag) requires
  a trait method.
- The Windows implementation must translate Win32 errors into
  `PlatformError` carefully.

### Trade-offs

- We accept a small translation cost (Win32 -> PlatformError) for the
  ability to test the orchestrator on every CI runner without an
  interactive Windows session.

## Alternatives

- **Inherent per-platform functions.** Rejected. Splits the code paths
  across modules and makes the orchestrator untestable.
- **Conditional compilation with `#[cfg(target_os)]`.** Rejected.
  Conflates the platform boundary with the build target and prevents
  the synthetic adapter from running on Windows.
- **Web platform abstraction (e.g. Capacitor).** Rejected. The product
  is desktop-first; web is not a constraint.
