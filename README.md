# PixelGrab

> Local-first Windows desktop capture and annotation utility.
> Windows v1 implementation in active hardening; not yet release-ready.

PixelGrab lets you turn anything visible on a Windows desktop into precise
visual context for an agent, browser, IDE, or collaborator. Capture a region,
add annotations, copy or save the result, or drag it directly into another
application. All processing happens locally.

## Status

The tracer implementation is present through tracer 15, but a 2026-08-22
production-wiring review found release-blocking gaps that isolated unit and
contract tests did not expose. See
[`docs/validation/2026-08-22-v1-gap-review.md`](docs/validation/2026-08-22-v1-gap-review.md)
for the current readiness assessment.

Implemented foundations include:

- `xcap`-backed Windows capture engine (`src-tauri/src/platform/windows/`)
  implementing the `PixelGrabPlatform` trait behind Windows Graphics Capture.
- Centralised coordinate conversion utilities in
  `pixelgrab_contracts::coordinate::transform` (client ↔ physical,
  physical ↔ capture buffer, capture buffer ↔ export).
- Frozen-frame retention so the commit pipeline can flatten without
  re-capturing; the flattened RGBA is the single source for the PNG
  and the bitmap-compatible clipboard representation.
- Overlay UI with dim mask, crosshair, and eight resize handles, wired
  to Ctrl+C commit and staged Escape behaviour via the
  `request_cancel` IPC.
- Structured capture diagnostics (`CaptureDiagnostics`) carrying
  capture-to-overlay latency and failure categorisation without ever
  recording pixels or clipboard content.
- Session orchestrator that rejects overlapping capture requests and
  returns to `Idle` deterministically on every exit path.

Multi-monitor capture, annotation, shelf, OLE drag, pin, cache, and settings
contracts exist, but some are not yet connected into complete packaged-app user
flows. Treat the gap review—not tracer issue state—as the release authority.

## Quickstart (Windows)

```powershell
# 1. Install dependencies (requires Node 20+ and Rust 1.77+)
pnpm install --frozen-lockfile

# 2. Run the synthetic capture quality gate
pnpm ci:rust
pnpm ci:check

# 3. Build the production binary
pnpm tauri:build
```

On a fresh checkout, the above must succeed without any undeclared local
dependencies. See [`AGENTS.md`](AGENTS.md) for the full command reference.

## Repository layout

```
pixelgrab/
├── AGENTS.md                  AI context (read this first)
├── README.md                  this file
├── LICENSE                    MIT
├── CONTRIBUTING.md            how to contribute
├── SECURITY.md                security policy
├── CODE_OF_CONDUCT.md         community standards
├── docs/
│   ├── ARCHITECTURE.md        architecture overview
│   ├── GLOSSARY.md            domain vocabulary
│   └── adr/                   architectural decisions
├── src/                       Svelte 5 frontend
├── src-tauri/                 Tauri 2 Rust backend
├── crates/                    shared Rust crates
│   ├── pixelgrab-contracts/   platform-neutral types
│   └── pixelgrab-test-support/  deterministic test adapters
├── tests/                     packaged-app acceptance tests
├── scripts/                   build + license scripts
├── package.json
├── pnpm-workspace.yaml
├── Cargo.toml                 Rust workspace
└── vite.config.ts
```

## Documentation

- [`AGENTS.md`](AGENTS.md) — entry point for AI agents and new contributors.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — architecture overview.
- [`docs/GLOSSARY.md`](docs/GLOSSARY.md) — domain vocabulary.
- [`docs/adr/`](docs/adr/) — architectural decision records.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — contribution guide.
- [`SECURITY.md`](SECURITY.md) — security policy.
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) — community standards.

## License

PixelGrab is MIT-licensed. See [`LICENSE`](LICENSE).
