---
name: pixelgrab-change
description: Implement, debug, or review PixelGrab behavior across Rust, IPC, and native windows with reproducible tests and explicit acceptance evidence. Use for behavior changes and code reviews; skip the workflow for simple prose or formatting edits.
---

# PixelGrab changes and reviews

Read [AGENTS.md](../../../AGENTS.md) and the relevant rows in
[verification.md](../../../docs/agents/verification.md). Resolve repository paths
from the Git root. Use existing authorization and available tools; no personal
skill, plugin, or parallel agent is required.

## Implement or debug

1. Identify the observable behavior and the owning module. For a bug, reproduce
   the trigger and state before changing code. If reproduction is unavailable,
   state the evidence and uncertainty; do not label a hypothesis confirmed.
2. Trace the affected user entrypoint through IPC registration, native operation,
   event delivery, and visible result. Read callers as well as implementations.
   Check the Cargo feature selection and actual window entrypoint when relevant.
3. Choose the smallest boundary that can prove the behavior with synthetic data.
   For a regression, make the test fail for the original reason before the fix
   where feasible. Mock external I/O, not the coordinator being verified.
4. Implement the complete operation in the owner. Exercise cancellation, injected
   failure after resource acquisition, and a subsequent successful operation.
   Check resource cleanup, state, and user-visible error delivery together.
5. Run focused tests, then affected quality gates from the verification guide.
   Check production compilation for feature/native changes. For UI wiring, test
   the actual entrypoint; list packaged or hardware behavior still unverified.

## Review

Pin a concrete base SHA (default to HEAD for local changes; for a branch use the
merge-base with its target). Include staged, unstaged, and new relevant files;
record the scope. Read the originating user request/issue and affected ADRs.

Review two independent questions, sequentially or with delegation if authorized:

- **Standards:** ownership, typed boundaries, privacy, deterministic seams,
  resource lifetimes, accessibility, and documented architectural rules.
- **Behavior:** requirements implemented through actual production callers,
  failure recovery, regression coverage, and truthful acceptance claims.

For each actionable finding give severity, file/line, concrete trigger, impact,
and missing evidence or proposed correction. Separate documented violations
from design suggestions. Prefer evidence over speculative abstractions. Do not
repeat formatter/linter findings that automation already reports.

## Close out

Report what changed, which commands ran and passed/failed, and what they prove.
Mark blocked or unrun checks explicitly. Include remaining native acceptance
gaps. A test filename, comment, mocked IPC response, early return, or title
assertion is not proof of its advertised user flow. Update enduring guidance
only when a new invariant or recurring failure warrants it.
