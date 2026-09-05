# Agent infrastructure ownership

`AGENTS.md` is the shared entrypoint. `CLAUDE.md` is a portable forwarding file
that instructs the agent to read it. The intended symlink was absent in this
checkout and creation required unavailable Windows privileges. The forwarding
file keeps fresh worktrees usable without elevation or duplicated instructions.
The checker also accepts a relative `AGENTS.md` symlink and warns about Git's
plain symlink placeholder when `core.symlinks=false`.

The canonical project skill lives in `.agents/skills/pixelgrab-change/` and is
also linked from AGENTS so agents without skill discovery can use it. The small
`.claude/skills/pixelgrab-change/SKILL.md` entrypoint routes to the same workflow.
Only these shared Claude files are unignored. `.claude/settings.local.json`,
worktrees, and `.codex/` runtime configuration remain local.

Machine-local notification hooks are not quality gates. Do not copy callback
ports, session IDs, personal skill dependencies, or permission grants into the
repository. There is no automatic stop hook that commits, pushes, or contacts
other services. Project checks run without an agent runtime or network access.

## Checks and optional Git hook

`pnpm agents:check` verifies shared instruction/skill links and the CLAUDE alias.
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
