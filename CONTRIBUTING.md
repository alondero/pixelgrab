# Contributing to PixelGrab

We welcome contributions. PixelGrab is in active development; the foundation
(tracer-01) is the entry point. Subsequent tracers deliver the real capture
pipeline, the annotation tools, the shelf, and the OLE drag.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By
participating you agree to abide by its terms.

## Getting started

1. Read [`AGENTS.md`](AGENTS.md). It defines the architecture, the
   vocabulary, and the testing seams.
2. Skim [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and
   [`docs/GLOSSARY.md`](docs/GLOSSARY.md).
3. Read the relevant ADRs in [`docs/adr/`](docs/adr/).
4. Fork the repository and create a topic branch.
5. Make your change.
6. Run every quality gate locally before opening a PR (see below).
7. Open a PR with a short description and a link to the issue.

## Branch naming

- `feature/<tracer>-<short-name>` for new capabilities.
- `fix/<short-name>` for bug fixes.
- `docs/<short-name>` for documentation-only changes.
- `chore/<short-name>` for tooling.

## Commit messages

Use the [Conventional Commits](https://www.conventionalcommits.org/)
format: `feat:`, `fix:`, `docs:`, `chore:`, `test:`, `refactor:`. The
commit subject must be 72 characters or fewer.

## Quality gates

Before opening a PR, run every gate locally:

```powershell
pnpm install --frozen-lockfile
pnpm ci:rust
pnpm ci:check
pnpm licenses:check
```

The CI pipeline runs the same gates plus a production build. A PR that
fails CI will not be merged.

## Branch protection on `main`

`main` is protected by the `Protect main - CI must pass` ruleset
(defined in `.github/rulesets/protect-main.json`). The ruleset enforces:

- **Required status checks** — `Frontend (lint, typecheck, test)` and
  `Rust (fmt, clippy, test)` must both pass. The `Build (production)`
  and `Packaged-app smoke test` jobs are intentionally **not** required:
  the CI graph is `install -> {rust, frontend} -> build -> e2e`, so a
  frontend failure leaves `build` and `e2e` skipped (not failed), which
  would deadlock the merge button.
- **Strict up-to-date branches** — the PR must be rebased onto the
  latest `main` before merge. Pushes that fall behind are blocked.
- **Pull request required** — direct pushes to `main` are blocked; every
  change goes through a PR.
- **No force pushes** — `non_fast_forward` rule blocks history rewrites
  on `main`.

The `.github/workflows/branch-protection-guard.yml` workflow runs on
every push to `main` and weekly, and fails CI if the ruleset is missing,
unenforced, or weakened. If a future change needs to alter the rules,
edit `.github/rulesets/protect-main.json`, apply it via the GitHub API,
and confirm the guard still passes.

## Tracer workflow

PixelGrab is delivered through "tracer" issues. Each tracer is a self-contained
slice that establishes or extends a piece of the architecture. A tracer is
considered complete when:

- The implementation matches the implementation plan in the issue.
- Every quality gate is green.
- The relevant ADRs are updated.
- The relevant sections of `AGENTS.md` are updated.
- A PR description links the issue and explains the deltas.

## Privacy

Never commit captured desktop content, real monitor topologies, or real
user files. Use the synthetic test adapters in
`crates/pixelgrab-test-support/` for anything that needs a framebuffer, a
monitor layout, a clock, or a filesystem root.

## Reporting vulnerabilities

See [`SECURITY.md`](SECURITY.md).
