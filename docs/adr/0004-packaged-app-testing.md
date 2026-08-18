# ADR-0004: Packaged-app testing strategy

## Status

Accepted (tracer-01).

## Context

PixelGrab's primary acceptance seam is the packaged Windows application.
The product spec (#12) requires tests that:

- Drive the same intents used by the tray and global shortcuts.
- Assert visible UI plus OS-observable results rather than private
  Svelte, Konva, or Rust state.
- Run with deterministic, non-private inputs.

A naive approach would launch the packaged app and click around, but
that approach:

- Captures real desktop content, which is a privacy violation.
- Is non-deterministic; CI would flake.
- Requires a Windows runner with a real desktop, which is expensive.

## Decision

We adopt a layered testing strategy:

1. **Rust unit tests** — exercise the orchestrator and the platform
   contract with the synthetic adapter. Fast and deterministic.
2. **Rust integration tests** — exercise the IPC contract, the session
   lifecycle, and the synthetic capture end-to-end. Deterministic. No
   real desktop.
3. **Frontend unit tests** — verify the TypeScript IPC types against
   the Rust serialised payloads. Run with Vitest.
4. **Frontend component tests** — verify the Svelte components render
   correctly and emit the expected events. Run with
   `@testing-library/svelte`.
5. **Golden image tests** — verify the synthetic capture pipeline
   produces the expected PNG. Run with `cargo test`.
6. **Packaged-app acceptance tests** — drive the packaged binary with
   WebdriverIO + the official Tauri service. Only the synthetic capture
   flow is exercised in CI. Real capture flows are tested
   interactively by the development team.

The deterministic test adapters in `pixelgrab-test-support` are the
**only** test adapters that may be used in CI. The synthetic capture
path is the only path allowed in CI.

Golden images are byte-for-byte compared for the parts of the canvas
that are pure raster. Annotation handles and selection strokes that
depend on the rendering engine use explicit tolerance.

## Consequences

### Positive

- Tests are deterministic and run in seconds.
- CI never sees real desktop content.
- The packaged-app seam is exercised by WebdriverIO with the official
  Tauri service, so the acceptance tests reflect the real binary.

### Negative

- Some pixel-level tests must use tolerance, which can obscure
  regressions.
- The WebdriverIO + Tauri service adds a heavyweight dependency to CI.

### Trade-offs

- We accept the WebdriverIO cost for the confidence that the
  packaged-app seam works.

## Alternatives

- **No packaged-app tests.** Rejected. The spec requires them.
- **Direct UI automation (e.g. WinAppDriver).** Rejected. The WebdriverIO
  - Tauri service is the official solution and is more portable.
- **End-to-end tests that capture the real desktop.** Rejected. Privacy
  violation.
