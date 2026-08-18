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
