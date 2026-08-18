# PixelGrab

> Local-first Windows desktop capture and annotation utility.
> Tracer-01: project foundation and runnable capture spine.

PixelGrab lets you turn anything visible on a Windows desktop into precise
visual context for an agent, browser, IDE, or collaborator. Capture a region,
add annotations, copy or save the result, or drag it directly into another
application. All processing happens locally.

## Status

This is the **tracer-01** build. It establishes the project foundation:

- Build system and tooling (Tauri 2, Rust, Svelte 5, TypeScript, Konva, Vite, pnpm).
- Test harnesses (Cargo, Vitest, golden-image, Tauri mock runtime).
- A synthetic capture pipeline that exercises the full Rust -> IPC -> Svelte
  -> Konva -> PNG path with no real desktop content.
- A single-instance tray and pre-allocated overlay window.
- Quality gates, CI workflow, and the documentation suite.

Subsequent tracers (issues #14 through #27) deliver the real Windows
capture, the annotation experience, the shelf, the OLE drag, and the pin
references.

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
