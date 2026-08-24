# Architectural decision records

This directory contains the architectural decision records (ADRs) for
PixelGrab. ADRs are immutable records of significant decisions. New
decisions are recorded as new files; superseded decisions are linked from
their successor.

## Template

Use the following template for new ADRs:

```markdown
# ADR-NNNN: <Title>

## Status

Proposed | Accepted | Superseded by ADR-XXXX

## Context

What is the issue we're seeing that motivates this decision?

## Decision

What is the decision we made?

## Consequences

What becomes easier? What becomes harder? What trade-offs are we accepting?

## Alternatives

What other options were considered? Why were they rejected?
```

## Index

- [ADR-0001](0001-tauri-svelte-konva-stack.md) — Tauri 2 + Svelte 5 + Konva stack
- [ADR-0002](0002-platform-contracts.md) — Platform contracts
- [ADR-0003](0003-physical-coordinate-ownership.md) — Physical-coordinate ownership
- [ADR-0004](0004-packaged-app-testing.md) — Packaged-app testing strategy
- [ADR-0005](0005-cache-and-shelf.md) — Cache and one-card shelf (tracer-07)
- [ADR-0006](0006-external-drag.md) — External drag (tracer-09)
- [ADR-0007](0007-cache-bounds-and-recovery.md) — Cache bounds + recovery (tracer-13)
- [ADR-0008](0008-text-blur-and-save-as.md) — Text, blur, and Save As (tracer-05)
- [ADR-0009](0009-revision-metadata.md) — Reopen / non-destructive revision metadata (tracer-10)
- [ADR-0010](0010-overlay-reveal-seam.md) — Single backend seam for the overlay reveal contract
- [ADR-0011](0011-v1-native-workflow-hardening.md) — v1 native workflow hardening (issue #63)
