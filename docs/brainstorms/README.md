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

## Related work artifacts

Design notes and implementation records that live outside the published
navigation:

- [`../ideation/2026-04-30-open-ideation.md`](../ideation/2026-04-30-open-ideation.md) — repo-grounded "surprise-me" ideation pass
- [`../plans/2026-03-23-claude-code-connection-docs-sandbox-verification.md`](../plans/2026-03-23-claude-code-connection-docs-sandbox-verification.md) — superseded Claude Code connection/sandbox plan
- [`../plans/2026-05-28-001-fix-overlay-rust-analyzer-replacement-plan.md`](../plans/2026-05-28-001-fix-overlay-rust-analyzer-replacement-plan.md) — drop the overlay `rust-analyzer` override
- [`../plans/2026-05-28-003-feat-mcp-request-dedup-caching-plan.md`](../plans/2026-05-28-003-feat-mcp-request-dedup-caching-plan.md) — MCP request deduplication and response caching (REV-008)
- [`../plans/2026-05-28-004-feat-expand-lsp-tool-surface-plan.md`](../plans/2026-05-28-004-feat-expand-lsp-tool-surface-plan.md) — expand the MCP LSP tool surface (REV-010)
- [`../plans/2026-05-28-005-fix-server-process-hardening-plan.md`](../plans/2026-05-28-005-fix-server-process-hardening-plan.md) — harden the directly-spawned server process (REV-011)
- [`../plans/2026-05-28-006-refactor-code-cleanup-backlog-plan.md`](../plans/2026-05-28-006-refactor-code-cleanup-backlog-plan.md) — superseded code cleanup backlog (REV-012)
- [`../plans/2026-06-01-001-fix-cold-start-workspace-match-plan.md`](../plans/2026-06-01-001-fix-cold-start-workspace-match-plan.md) — cold-start workspace match fix
- [`../residual-review-findings/feat-expand-lsp-tool-surface.md`](../residual-review-findings/feat-expand-lsp-tool-surface.md) — residual review findings for REV-010
- [`../solutions/build-errors/overlay-rust-analyzer-replacement-cache-miss.md`](../solutions/build-errors/overlay-rust-analyzer-replacement-cache-miss.md) — overlay replacement cache-miss write-up
