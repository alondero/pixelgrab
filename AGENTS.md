# PixelGrab agent guide

PixelGrab is a local-first Windows capture and annotation app: one resident
Tauri process, Svelte 5 runes, Konva, and a Rust core. Work from the repository
root. Read this guide first, then only references relevant to your change.
`CLAUDE.md` points here; maintain one source of instructions.

## Start and finish a change

- Inspect `git status --short` and the current diff; preserve unrelated work.
- Establish the requested behavior from the user or issue, including observable
  acceptance criteria. The tracker is [alondero/pixelgrab](https://github.com/alondero/pixelgrab/issues).
- For behavior changes, bug fixes, or reviews, use the repository
  [pixelgrab-change skill](.agents/skills/pixelgrab-change/SKILL.md).
  It works without personal skills, plugins, or subagents.
- Read [architecture](docs/ARCHITECTURE.md), [glossary](docs/GLOSSARY.md),
  and the controlling [ADRs](docs/adr/README.md) for the affected boundary.
  ADRs govern architectural intent; inspect code to establish actual behavior.
- Use the [verification guide](docs/agents/verification.md) to choose tests.
  Record commands actually run, results, and remaining acceptance gaps.
- Keep changes within the task. Commit, push, publish, and issue mutations follow
  the user's authorization; a personal finish skill does not authorize them.

## Boundaries and invariants

- `crates/pixelgrab-contracts/` owns platform-neutral contracts and pure logic.
  `src-tauri/src/platform/` owns Windows operations behind `PixelGrabPlatform`;
  test adapters live in `crates/pixelgrab-test-support/`.
- Frontend IPC goes through `src/lib/ipc/`; mirror Rust wire changes in
  TypeScript and test both sides. Use structured `PlatformError` / `thiserror`.
- One backend operation should own an ordered native workflow. Overlay reveal
  uses `overlay::show_over_virtual_desktop` (ADR-0010, amended by ADR-0011).
  Backend-to-backend actions must not depend on a hidden WebView relaying events.
- Preallocated windows outlive captures: handle subsequent capture-ready events,
  rehydration, and listener cleanup. Static Tauri windows declare their `url`.
- Coordinates crossing boundaries must identify physical desktop, crop-local,
  or WebView units. Cover negative origins and differing scale factors (ADR-0003).
- Session terminal paths must release their resources and permit the next
  operation, including cancellation and partial failure. The cache owns durable
  assets and lock lifetimes; consumers use that registry (ADRs 0005–0007, 0011).
- Keep frozen source pixels immutable. Every export applies annotations through
  the shared flatten pipeline; revision metadata alone does not prove a
  non-destructive edit round trip (ADRs 0008–0009).
- Cargo `default = []` must stay empty. Production Windows builds select real
  capture and OS hotkeys; tests explicitly enable `synthetic`. Preserve
  `custom-protocol`. See `src-tauri/Cargo.toml` and `default_platform` in `lib.rs`.

## Standards

- Rust: public APIs have `///` docs; use `cargo fmt` and warning-free clippy.
  `unsafe` is restricted to platform modules with a `// SAFETY:` explanation.
- TypeScript: strict types, no `any`; Svelte 5 runes, no legacy `$:`. Components
  live in `src/lib/<feature>/`; shared reactive state uses `*.svelte.ts`.
  Tests live beside frontend code. ESLint and Prettier must pass.
- Put policy in the module owning the invariant. Introduce a seam for a real
  external dependency or lifecycle, not a second coordinator or test-only mirror.
- Follow [accessibility](docs/ACCESSIBILITY.md): keyboard operation, visible
  focus, labelled controls, state conveyed beyond colour, Windows text scaling.
- Update the relevant ADR for architectural changes using its template; retain
  superseded ADRs with forward links. Keep this entrypoint short and put detail
  beside the affected module or in linked documentation.

## Privacy and deterministic tests

Never log pixels, annotation text, clipboard content, settings secrets, or paths
outside the application cache. IPC errors use categorical diagnostics without
raw external paths. Scrub failure artifacts before sharing them.

Tests use synthetic capture/layout, controllable clocks, and isolated filesystems
from `pixelgrab-test-support`. CI must never trigger real desktop capture. A
production startup smoke test may start the process but must not request capture.
Native acceptance uses a controlled desktop and records only sanitized evidence.

## Commands

| Purpose                                             | Command                                       |
| --------------------------------------------------- | --------------------------------------------- |
| Frozen dependency install                           | `pnpm install --frozen-lockfile`              |
| Agent infrastructure check (no dependencies needed) | `node scripts/check-agent-infra.mjs`          |
| Frontend format, lint, types, tests                 | `pnpm ci:check`                               |
| Rust format, clippy, tests with synthetic adapter   | `pnpm ci:rust`                                |
| Both source suites                                  | `pnpm ci:all`                                 |
| License policy                                      | `pnpm licenses:check`                         |
| Frontend build                                      | `pnpm build`                                  |
| Production Windows build                            | `pnpm tauri:build`                            |
| Synthetic Rust tests only                           | `cargo test --workspace --features synthetic` |
| Develop frontend / native shell                     | `pnpm dev` / `pnpm tauri:dev`                 |

Rust and frontend checks are independent. `pnpm ci:all` does not include the
production build, licenses, dependency audit, or packaged acceptance. Bare
`cargo test --workspace` is not the Windows test workflow.

## Evidence and navigation

Green unit tests and a surviving process do not establish a working capture flow.
The v1 sign-off was withdrawn; [the latest checked-in workflow review](docs/validation/2026-09-01-v1-workflow-review.md)
records outstanding packaged, revision, settings, and hardware acceptance gaps.
Verify their current status before claiming completion of a release story.

- [Verification and failure-path matrix](docs/agents/verification.md)
- [Infrastructure ownership and hook usage](docs/agents/infrastructure.md)
- [Infrastructure audit and review evidence](docs/agents/2026-09-05-audit.md)
- [Historical implementation inventory](docs/agents/implementation-reference.md)
  (read selectively; historical descriptions are not current acceptance evidence)
