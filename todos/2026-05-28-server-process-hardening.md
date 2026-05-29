---
status: planned
priority: p2
issue_id: "REV-011"
tags: [security, bootstrap, process-lifecycle, sockets]
dependencies: []
plan: "docs/plans/2026-05-28-005-fix-server-process-hardening-plan.md"
---

# Harden the directly-spawned server process and socket directory

Consolidates SEC-4 and SEC-5 from `review-2026-03-18-p2-important.md` (archived
under `todos/archive/`).

## Problem Statement

The direct-spawn bootstrap path and the socket directory it relies on have two
loose ends: orphaned server processes can accumulate, and on some configurations
the socket is world-accessible.

## Findings (from review)

- **SEC-4 (detached server, no PID tracking)** (`mcp-server/src/bootstrap.rs`,
  `start_direct_server()`): spawns `lspmux server` with null stdio, then drops
  the child handle. No `kill_on_drop`, no PID stored. Concurrent invocations can
  race multiple servers onto one socket; orphans accumulate over time.
- **SEC-5 (socket dir permissions)** (`setup`): `mkdir -p` leaves the socket
  directory at default 0755. With `XDG_RUNTIME_DIR` unset and `TMPDIR=/tmp`, the
  socket at `/tmp/lspmux/lspmux.sock` is world-accessible; other local users
  could connect and issue LSP commands.

## Proposed approach

- SEC-4: write a pidfile alongside the socket; check it before spawning; log to a
  file instead of `/dev/null`. Consider `kill_on_drop` where lifetime allows.
- SEC-5: `chmod 700 "${LSPMUX_SOCKET_DIR}"` after `mkdir -p`.

## Acceptance Criteria

- [ ] Direct-spawn writes and checks a pidfile; no duplicate servers on one socket
- [ ] Spawned server logs to a file, not `/dev/null`
- [ ] Socket directory is created mode 0700
- [ ] Regression tests/fixtures cover the pidfile and permission invariants

## Work Log

### 2026-05-28 - Promoted from review snapshot

**By:** Claude Code (todos/brainstorms consolidation)

**Actions:**
- Consolidated SEC-4 and SEC-5 from the archived P2 review into one hardening
  todo.
