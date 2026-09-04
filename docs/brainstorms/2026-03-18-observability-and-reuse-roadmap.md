# Observability and artifact-reuse roadmap

**Date:** 2026-03-18
**Status:** Phase 1 is complete. Phase 2 is partially implemented and tracked
by the [REV-005 plan](../plans/2026-05-28-002-feat-compiler-action-reuse-accounting-plan.md).

## Purpose

One shared rust-analyzer per worktree is necessary, but it does not by itself
show that clients avoided duplicate compiler work. The runtime should make the
service, client, readiness, and compiler-accounting boundaries visible without
requiring log archaeology.

| Phase | State | Deliverable |
|---|---|---|
| 1. Runtime observability | Complete | Client identity, bootstrap/tool telemetry, server status, workspace registry, and `experimental/serverStatus` readiness ingestion |
| 2. Compiler accounting | Partial | `rust_server_status` parses the most recent local `target/flycheck*/stdout` cargo-JSON file; REV-005 defines the remaining attributable accounting work |
| 3. Validation | Pending Phase 2 | Prove the one-service invariant and accounting semantics with hermetic tests; add local inspection only when it answers an operational question |

## Runtime contract

For a worktree, `rust_server_status` and `rust_workspace_registry` should make
these questions answerable:

- Is the lspmux daemon reachable, and has it created the requested workspace
  instance?
- Is the MCP client's rust-analyzer transport alive and is rust-analyzer
  quiescent, warning, or unhealthy?
- Which stable client identity started this MCP process?
- What bootstrap and tool outcomes has this MCP process recorded?
- What cargo-JSON artifact data was found locally, and what are its limits?

The last item is deliberately narrower than a claim about cache efficiency.
The current scan is a snapshot of a local flycheck output file, not a causal
record of a client request and not a measurement of direct-editor Cargo work.

## Phase 2 boundary

REV-005 owns the next decision and implementation work. Its minimum useful
outcome is an accounting stream for rust-analyzer-induced Cargo activity that
can distinguish `fresh` from rebuilt artifacts and identify the workspace and
client context. A wrapper or captured Cargo JSON may be suitable; external
process observation is only a validation aid because attribution is weak.

`sccache` remains a cooperating cache layer. Configuring it, sharing target
directories across worktrees, and CI cache warming are not lspmux-cc product
work. In particular, a shared target directory is not a default: Cargo locking
can erase the concurrency benefit the project is intended to preserve.

## Validation after accounting exists

- Exercise two clients against one worktree and verify one rust-analyzer
  instance through the registry/integration harness.
- Feed known Cargo JSON through the accounting path and verify the status
  schema and fresh/rebuilt totals.
- Verify that the recorded workspace and client context are not inferred from
  unrelated process activity.

## References

- [REV-005 plan](../plans/2026-05-28-002-feat-compiler-action-reuse-accounting-plan.md)
- [local-cache disposition](archive/2026-03-18-local-dev-cache-optimization.md)
