# Todos

Tracked, actionable work items for lspmux-cc. One file per item, started from
[`assets/todo-template.md`](assets/todo-template.md). Completed work and triaged
review snapshots live under [`archive/`](archive/).

## Open

| Todo | Priority | Summary |
|------|----------|---------|
| [REV-005: compiler / artifact-reuse accounting](2026-03-18-compiler-action-and-artifact-reuse-accounting.md) | p1 | Measure when rust-analyzer forces compiler work vs reuses artifacts (REV-004 groundwork done) |
| [REV-010: expand LSP tool surface](2026-05-28-expand-lsp-tool-surface.md) | p2 | Code actions, rename, document symbols, readiness, call hierarchy, go-to-impl, expand-macro |
| [REV-011: server process hardening](2026-05-28-server-process-hardening.md) | p2 | PID tracking for direct-spawn; 0700 socket dir |
| [REV-012: code cleanup backlog](2026-05-28-code-cleanup-backlog.md) | p3 | Opportunistic simplification / perf / pattern cleanup |

## Archived

Completed items (with evidence pointers in their frontmatter) and the triaged
2026-03-18 review snapshots, preserved for audit:

- [`archive/2026-03-18-observability-and-client-attribution.md`](archive/2026-03-18-observability-and-client-attribution.md) (REV-004, done)
- [`archive/2026-03-18-linux-hook-bootstrap-parity.md`](archive/2026-03-18-linux-hook-bootstrap-parity.md) (REV-006, done)
- [`archive/2026-03-18-crate-replacement-opportunities.md`](archive/2026-03-18-crate-replacement-opportunities.md) (REV-007, done)
- [`archive/2026-03-18-shell-error-suppression-audit.md`](archive/2026-03-18-shell-error-suppression-audit.md) (REV-009, done)
- [`archive/review-2026-03-18-p1-critical.md`](archive/review-2026-03-18-p1-critical.md) (review snapshot; see its disposition table)
- [`archive/review-2026-03-18-p2-important.md`](archive/review-2026-03-18-p2-important.md) (review snapshot)
- [`archive/review-2026-03-18-p3-cleanup.md`](archive/review-2026-03-18-p3-cleanup.md) (review snapshot)
