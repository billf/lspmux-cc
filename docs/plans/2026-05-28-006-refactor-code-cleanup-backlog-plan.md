---
title: "Code cleanup backlog (REV-012)"
status: superseded
date: 2026-05-28
audited: 2026-09-04
type: refactor
issue_id: REV-012
origin: todos/2026-05-28-code-cleanup-backlog.md
---

# Code cleanup backlog (REV-012)

## Disposition

Superseded as a bulk refactor. The original twelve-item checklist mixed proven
defects, already-completed work, speculative micro-optimizations, and style
preferences. Applying it as one change would make review harder while offering
no coherent user-facing outcome.

The highest-value structural cleanup from the list—the expanded MCP tool surface
and its shared response shaping—has already landed in focused commits. The
remaining items should be proposed only with a measured or correctness-based
reason, in the owning change, rather than revived as a sweep.

## Audit ledger

| Item group | Disposition | Reason |
|---|---|---|
| `const fn` candidates | closed | Existing useful helpers are already `const`; `internal_error` must allocate an owned MCP error. |
| Language-ID trimming | closed | The mapping intentionally supports common non-Rust files; reducing it would be a regression. |
| 100 MiB message limit | closed | It is a deliberate defensive limit with regression coverage, not unmeasured cleanup. |
| `OnceCell` conversion | deferred | It changes initialization semantics; make it only with a demonstrated lock-path cost and explicit duplicate-set behavior. |
| Coalesced LSP writes | deferred | Benchmark first; preserve framing and cancellation behavior if a change is justified. |
| Derive removal | closed | Output schemas and focused tests use several formerly suspected derives. Remove a derive only when a concrete type proves it redundant. |
| Generic tool preamble | closed | Tool-specific validation and error context are clearer than an abstraction that merely joins two calls. |
| Async path metadata | deferred | A local metadata check is not a demonstrated async-runtime bottleneck. |
| Stdlib-only tests | closed | Retain tests that exercise the client’s synchronization and lifecycle invariants, not just the standard-library primitive. |
| Shell error-prefix normalization | closed | Cosmetic-only; do it opportunistically when touching a script. |
| Binary-resolution documentation | delivered elsewhere | Host guides document supported configuration and overrides; a separate exhaustive cascade would become stale quickly. |
| `sed` replacement concern | closed | The setup substitution uses a safe delimiter with repository-controlled replacement data. |

## Follow-up rule

A future cleanup must name the observable problem, keep one behavioral concern
per change, and include a focused regression or benchmark. Do not reopen
REV-012 as a broad mechanical refactor.
