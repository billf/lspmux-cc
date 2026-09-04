# Brainstorms

Exploratory design notes for lspmux-cc. Notes that graduated into committed work
are promoted to implementation plans under [`../plans/`](../plans/); superseded
notes live in [`archive/`](archive/).

## Live

| Doc | Summary |
|-----|---------|
| [observability & artifact-reuse roadmap](2026-03-18-observability-and-reuse-roadmap.md) | Phase 1 is complete; REV-005 owns the remaining attributable compiler-accounting work |
| [MCP server testability boundary](2026-05-04-mcp-server-workspace-split-requirements.md) | Retires the unjustified four-crate split; the remaining gap is making the existing tools module library-testable |

## Archived

- [`archive/2026-02-05-lspmux-claude-code-brainstorm.md`](archive/2026-02-05-lspmux-claude-code-brainstorm.md) (superseded by M1-M5)
- [`archive/2026-03-18-per-worktree-socket-routing.md`](archive/2026-03-18-per-worktree-socket-routing.md) (superseded: upstream lspmux already multiplexes worktrees)
- [`archive/2026-03-18-local-dev-cache-optimization.md`](archive/2026-03-18-local-dev-cache-optimization.md) (outside this repository's scope; the measurable portion belongs to REV-005)

See [`../migration-m5.md`](../migration-m5.md) for the on-demand-spawn model that
replaced the launchd/systemd auto-start assumption.
