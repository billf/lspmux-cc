# Local development cache optimization

**Date:** 2026-03-18
**Status:** Superseded 2026-09-04. The actionable, product-owned part is
tracked by the [REV-005 plan](../../plans/2026-05-28-002-feat-compiler-action-reuse-accounting-plan.md).

## Disposition

This note mixed two concerns:

- **Product evidence:** whether rust-analyzer-induced cargo work reused or
  rebuilt artifacts. This is in scope and belongs to REV-005.
- **Developer-machine cache configuration:** selecting, installing, or fixing
  `sccache`, sharing target directories, and warming CI caches. This is local
  infrastructure, not lspmux-cc product behavior.

The former now has a scoped implementation plan. The repository already
exposes a best-effort snapshot by parsing the newest local
`target/flycheck*/stdout` cargo-JSON file through `rust_server_status`; it is
not yet an attributable, process-owned accounting stream. REV-005 is the
place to decide whether and how to close that gap.

## Retained guidance

- Keep rust-analyzer worktrees isolated at the Cargo `target/` level. A shared
  target directory trades cache reuse for Cargo lock contention and is not a
  lspmux-cc design goal.
- Treat `sccache` as a cooperating external cache. Its hit/miss statistics do
  not prove that a particular rust-analyzer request reused Cargo artifacts.
- Do not use transient local build artifacts or a developer's environment as
  repository-wide evidence that `sccache` is broken. Reproduce and document
  any such issue in the affected development environment.

## References

- [REV-005 implementation plan](../../plans/2026-05-28-002-feat-compiler-action-reuse-accounting-plan.md)
- [observability and artifact-reuse roadmap](../2026-03-18-observability-and-reuse-roadmap.md)
