# Verification by boundary

Choose checks for the behavior being changed. A test must assert an observable
result and fail when the relevant behavior breaks. Do not substitute source-text
searches or tests that reproduce the implementation for behavioral assertions.
Static configuration tests remain useful for configuration invariants.

| Change                         | Start at these seams                                                                                                                         | Evidence to obtain                                                                                                                                                      |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Session / overlay              | `src-tauri/tests/session_lifecycle.rs`, `src/lib/overlay/OverlayApp.test.ts`, `src-tauri/tests/window_config.rs`                             | Capture-ready after initial mount and on second capture; correct entrypoint; terminal cleanup; next capture succeeds. Native visibility needs packaged evidence.        |
| IPC payload / command / event  | `src-tauri/tests/ipc_contracts.rs`, `src/lib/ipc/types.test.ts`, `src/lib/ipc/shell.test.ts`, `src-tauri/src/lib.rs`                         | Rust/TS round trip, command registration, actual producer and consumer, event payload and unsubscribe behavior.                                                         |
| Geometry / annotations         | `src/lib/overlay/coordinates.test.ts`, `src/lib/overlay/KonvaStage.test.ts`, contracts annotation tests, `src-tauri/tests/golden_capture.rs` | Physical export dimensions, crop-local transforms, negative origins, differing scales, blur on every export, undo as complete actions.                                  |
| Cache / shelf / timers         | `src-tauri/tests/cache_atomic.rs`, `src-tauri/tests/shelf_queue_integration.rs`, `src/shelf.test.ts`, `src/lib/shelf/`                       | Publish after assets; no phantom card on failure; targeted removal; restore on startup; shared clock epoch; hover/expiry; lock release.                                 |
| Pin / drag / revision          | `src-tauri/tests/pin_lifecycle.rs`, `src-tauri/tests/revision_round_trip.rs`, `src-tauri/src/platform/windows/drag.rs`                       | Locks through native lifetime; failure/cancel cleanup; COM ownership; immutable source plus editable scene. External drop and TopMost interaction need native evidence. |
| Features / startup / packaging | `src-tauri/Cargo.toml`, `src-tauri/tests/tauri_config.rs`, `src-tauri/tests/window_config.rs`, CI build and smoke jobs                       | Synthetic tests plus production compilation. Confirm real platform/hotkey selection; startup survival proves startup only.                                              |
| Agent docs / skills / hooks    | `scripts/check-agent-infra.mjs`, `scripts/check-agent-infra.test.mjs`                                                                        | Entrypoints resolve, local links exist, shared resources survive a fresh checkout, check failures return nonzero.                                                       |

## Practical commands

Run from the root after `pnpm install --frozen-lockfile`:

```powershell
pnpm exec vitest run src/lib/overlay/OverlayApp.test.ts
cargo test -p pixelgrab --features synthetic --test session_lifecycle
pnpm ci:check
pnpm ci:rust
pnpm licenses:check
pnpm build
```

Select the focused test for your change, then run the affected full suite. Run both
for IPC and cross-layer changes. Native/configuration changes additionally require
`pnpm tauri:build` on Windows. Do not enable `synthetic` in Cargo defaults to fix
a test build. Documentation-only edits need infrastructure and formatting checks;
hook/checker edits also need `node --test scripts/check-agent-infra.test.mjs` and
ESLint. If a required tool or environment is unavailable, report that limitation.

## Native acceptance is a separate claim

At this audit, `tests/e2e/` is scaffolding: WebDriver dependencies and a runnable
package command are absent, CI does not execute those specs, and the full-desktop
cases contain placeholder assertions. Setting `PIXELGRAB_E2E_FULL=1` does not
turn them into a capture-to-delivery test. Do not cite them as completed coverage.

For a future automated packaged workflow, provision an explicitly synthetic
binary and isolated cache; drive capture → select → annotate → commit → visible
shelf through the actual UI, then a second capture. Assert exported dimensions
and content using only synthetic pixels. Verify drag, pin, and reopen with
controlled native fixtures. Keep the production startup job separate so real
backend selection remains covered without taking desktop captures in CI.

For manual native acceptance, record build SHA, build command/features, Windows
and display-scale setup (sanitized), trigger, expected result, observed result,
and remaining limitations. Never upload real captures or unsanitized logs.

## Evidence format

Use this in the PR or task result; omit inapplicable rows rather than inventing
checks. A failed or unrun check stays visible.

| Behavior / risk                       | Command or manual procedure                                 | Result                                 | What remains unproven                       |
| ------------------------------------- | ----------------------------------------------------------- | -------------------------------------- | ------------------------------------------- |
| Concrete trigger and expected outcome | Exact command; test file and test name for a coverage claim | Passed / failed / not run, with reason | Native, hardware, or integration limitation |

For a regression, record whether the test failed before the fix and why. If it
could not be run against the baseline, say so. Test counts alone are not coverage.
