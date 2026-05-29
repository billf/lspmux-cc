# Brainstorms

Exploratory design notes for lspmux-cc. Notes that graduated into committed work
are tracked in [`../../todos/`](../../todos/) or under `docs/`; superseded notes
live in [`archive/`](archive/).

## Live

| Doc | Summary |
|-----|---------|
| [observability & artifact-reuse roadmap](2026-03-18-observability-and-reuse-roadmap.md) | Phase 1 (observability) shipped via REV-004; Phase 2 (compiler accounting, REV-005) and Phase 3 (validation) open |
| [local dev cache optimization](2026-03-18-local-dev-cache-optimization.md) | sccache + artifact sharing around a shared rust-analyzer; feeds REV-005 |
| [mcp-server workspace split](2026-05-04-mcp-server-workspace-split-requirements.md) | 4-crate split for testability; subsumes review ARCH-1 / ARCH-3 |

## Archived

- [`archive/2026-02-05-lspmux-claude-code-brainstorm.md`](archive/2026-02-05-lspmux-claude-code-brainstorm.md) (superseded by M1-M5)
- [`archive/2026-03-18-per-worktree-socket-routing.md`](archive/2026-03-18-per-worktree-socket-routing.md) (superseded: upstream lspmux already multiplexes worktrees)

See [`../migration-m5.md`](../migration-m5.md) for the on-demand-spawn model that
replaced the launchd/systemd auto-start assumption.
