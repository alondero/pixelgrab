# Agent infrastructure ownership

`CLAUDE.md` is the canonical shared entrypoint. `AGENTS.md` is a relative
symlink to it. The `.claude/` skill is canonical and `.agents/` is a relative
symlink to that skill, so Claude-oriented files are the maintained source and
agent-oriented names remain compatible with common repository conventions. When
Windows checks out symlinks as plain files (`core.symlinks=false`), the checker
accepts and validates the target placeholder.

The canonical project skill lives in `.claude/skills/pixelgrab-change/` and is
symlinked from `.agents/skills/pixelgrab-change/` so agents without Claude skill
discovery can use the same workflow. Only these shared Claude files are
unignored. `.claude/settings.local.json`,
worktrees, and `.codex/` runtime configuration remain local.

Machine-local notification hooks are not quality gates. Do not copy callback
ports, session IDs, personal skill dependencies, or permission grants into the
repository. There is no automatic stop hook that commits, pushes, or contacts
other services. Project checks run without an agent runtime or network access.

## Checks and optional Git hook

`pnpm agents:check` verifies shared instruction/skill aliases and the CLAUDE guide.
CI invokes it in the frontend job; `pnpm ci:check` invokes it locally. The checker
does not validate prose semantics or establish product acceptance.

The tracked `.githooks/pre-commit` runs this fast dependency-free check. Git does
not install tracked hooks automatically. To use it for an authorized commit
without changing configuration shared with other worktrees:

```powershell
git -c core.hooksPath=.githooks commit
```

The hook checks working-tree infrastructure, not a snapshot of the index, so
partially staged infrastructure changes still need review. CI checks the actual
commit. It does not run the full Rust build on every edit or stage files. Existing
machine hook configuration is left intact. Set executable mode on the hook in
Git when adding it so Unix checkouts can run it.

## Maintaining this scaffold

Add durable behavioral lessons to the verification matrix or owning ADR. Keep
the top-level guide navigable; keep historical implementation detail in the
reference inventory. Review claims against source and actual test assertions.
For new shared skills, add them to the checker's document list and register a
discovery entrypoint only for clients that need one.
