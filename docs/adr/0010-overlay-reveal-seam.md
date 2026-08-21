# ADR-0010: Single backend seam for the overlay reveal contract

## Status

Accepted (tracer-15 follow-up, GitHub issue #60). Replaces the two-step
reveal split introduced during tracer-01 (overlay mount + state advance
via separate IPC).

## Context

The overlay reveal contract was historically split across two Rust
calls and one frontend call:

1. `request_capture` positioned the overlay window.
2. `OverlayApp.svelte` mounted and called `requestOverlay` to walk the
   orchestrator from `Ready` to `Selecting`.
3. A defensive `reset()` in `request_capture` covered the case where
   the overlay failed to mount.

This split made every change to "show the freeze frame" require edits
to five files. PR #58 fixed the immediate v1 release blocker (the
session stuck in `Ready` because step 2 was being skipped) but did not
collapse the seam itself. The two-axis review flagged the resulting
Leaky Abstraction / Shotgun Surgery smell as the single underlying
cause.

The shape after PR #58 also had latent gaps: `show_over_virtual_desktop`
was defined but had no caller, so the overlay window was positioned
but never actually shown. `request_overlay` was wired into the IPC
layer but had no frontend caller, so it was dead code.

## Decision

Make `crate::overlay::show_over_virtual_desktop(app, layout, session)`
the single backend seam for the overlay reveal contract. The function:

1. Positions the overlay window over the captured bounds (delegating
   to `position_over_virtual_desktop` / `position_over_bounds`).
2. Calls `window.show()` so the freeze frame becomes visible.
3. Walks the orchestrator from `Ready` to `Selecting` via
   `SessionOrchestrator::overlay_mounted`, stamping the
   capture-to-overlay latency on the stored diagnostics.

`request_capture` is the only caller of `show_over_virtual_desktop`.
`OverlayApp.svelte` only reads `getSessionSnapshot` to render the
freeze frame — it no longer drives any state-machine step on mount.

`SessionOrchestrator::overlay_mounted` is a no-op from any state
other than `Ready`. The orchestrator's existing `request_transition`
already enforces the legal-edge invariant, so an out-of-order reveal
(from a duplicate mount, an in-flight cancel, or any non-Ready state)
is a silent no-op rather than an error. The overlay window is still
shown — only the state machine stays put.

The dead code is deleted: `request_overlay` IPC handler,
`RequestOverlayIntent` / `RequestOverlayResult` / `OverlaySelection`
wire types, the `requestOverlay` TS wrapper, the
`mockRequestOverlay` Vitest fixture, and the `overlay_visible` /
`begin_selecting` orchestrator methods are all removed. The
`mockRequestCapture` Vitest fixture now walks Idle → Selecting in one
call to mirror the new backend seam.

The defensive `reset()` is not replaced. The no-op semantics of
`overlay_mounted` mean a stuck `Ready` recovers naturally the next
time a capture positions the overlay — `reset()` was masking a
symptom that no longer exists.

## Consequences

### Positive

- The `Ready → Selecting` transition has exactly one Rust call site.
- The frontend no longer imports `requestOverlay` or any overlay-state
  IPC; `OverlayApp.svelte` is a pure render over the session
  snapshot.
- The overlay window is reliably visible after a capture (PR #58
  left the `show_over_virtual_desktop` function defined-but-uncalled;
  this ADR wires the missing caller).
- New tests cover both branches: `overlay_mounted_walks_ready_to_selecting_and_stamps_diagnostics`
  exercises the Ready → Selecting path, and
  `overlay_mounted_is_noop_from_non_ready_states` covers the no-op
  semantics.

### Negative

- A consumer that previously called `request_overlay` from outside
  the IPC layer (none found in the tree at the time of writing) would
  need to fold its logic into `show_over_virtual_desktop` or call
  `overlay_mounted` directly. The orchestrator's `overlay_visible`
  and `begin_selecting` aliases are removed, so any such consumer
  fails to compile until migrated.
- The orchestrator's `overlay_mounted` swallows `request_transition`
  errors via `let _ =` only when the state is not `Ready`. Inside
  the `Ready` branch, transition failures propagate as
  `PlatformError::InvalidSessionState`, matching the orchestrator's
  error-propagation contract. A cancelled overlay that races the
  transition still surfaces a real error.

### Trade-offs

- We accept the loss of the `request_overlay` IPC as a public surface.
  Any future "report selection" wire contract would need to be
  re-introduced deliberately, not inherited.

## Alternatives

- **Keep the two-step contract and fix `request_overlay` to be the
  single frontend entry point.** Rejected. The frontend doesn't need
  to know about the state-machine step at all; collapsing the seam
  removes the leak rather than working around it.
- **Move the state transition to the frontend.** Rejected. The
  orchestrator is the canonical owner of the state machine; the
  frontend is a render layer.
- **Replace the no-op semantics with a panic / log-on-stuck.**
  Rejected. The overlay window is the user-visible artefact; failing
  the IPC because of a transient race would be worse than silently
  leaving the orchestrator in its current state.

## Related

- AGENTS.md §3 (Architecture) and §5 (Capture-session lifecycle) for
  the navigation-aid updates that reflect this collapse.
- `overlay/mod.rs::show_over_virtual_desktop` and
  `session/state.rs::overlay_mounted` for the implementation.
- `src-tauri/tests/session_lifecycle.rs::capture_session_walks_full_lifecycle`
  and the inline tests in `session/state.rs` for the regression tests.
