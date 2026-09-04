---
title: "fix: Harden the directly-spawned server process (REV-011)"
status: active
date: 2026-05-28
type: fix
issue_id: REV-011
origin: todos/2026-05-28-server-process-hardening.md
---

# fix: Harden the directly-spawned server process (REV-011)

## Summary

The direct-spawn bootstrap path (`start_direct_server()` in `mcp-server/src/bootstrap.rs`) spawns `lspmux server` with null stdio and immediately drops the child handle. Concurrent invocations can race, and failures are hard to diagnose. This plan needs an endpoint-agnostic spawn lease plus file logging. A pidfile alone is not a single-instance guard: check-then-spawn and atomic pidfile replacement still permit two callers to spawn. **SEC-5 (Unix socket directory 0700) is already fixed** — `setup` runs `chmod 700 "${LSPMUX_SOCKET_DIR}"` — so this plan is scoped to SEC-4 and a regression guard.

---

## Problem Frame

- **SEC-4 (open):** `start_direct_server()` builds `lspmux server --config <path>`, sets `stdout`/`stderr` to `Stdio::null()`, calls `.spawn()`, and drops the returned child. There is no spawn lease, PID record, or `kill_on_drop`. Concurrent bootstraps can spawn duplicate servers for the same endpoint; spawned-server output is discarded (`/dev/null`), so failures are undiagnosable.
- **SEC-5 (already resolved):** the review flagged the socket directory at default 0755 (world-accessible socket under `/tmp/lspmux/`). Verification shows `setup` already creates the dirs and runs `chmod 700 "${LSPMUX_SOCKET_DIR}"` (`setup:46-47`). No code change needed; add a regression guard so it can't silently regress.

Socket dir precedence (verified, `setup:13-23`): `XDG_RUNTIME_DIR` → `TMPDIR` → `/tmp`, with the lspmux dir under `RUNTIME_BASE`.

---

## Requirements

- **R1.** Direct-spawn uses an endpoint-agnostic, atomic spawn lease; concurrent callers either observe the ready service or wait for the lease holder, rather than spawning independently.
- **R2.** The spawned server logs to a file, not `/dev/null`.
- **R3.** Socket directory is created mode 0700. *(Already satisfied at `setup:47`; this plan adds a regression guard, not a new fix.)*
- **R4.** Regression tests/fixtures cover the spawn-lease and permission invariants.

---

## Key Technical Decisions

**KTD1 — Atomic spawn lease, with pid as diagnostic data.** Acquire a lock or
create-new lease file before spawning; other callers poll the configured endpoint
and wait briefly for the holder to finish. The lease key derives from the resolved
connection endpoint, not a Unix-socket directory, because TCP is supported too.
After readiness succeeds, persist a PID only as diagnostic/stale-lease recovery
information. Define the stale-holder rule and cleanup before implementation.

*Rationale:* A pidfile records a process but does not serialize check-and-spawn.
An atomic lease does. It also works for TCP endpoints, where “beside the socket”
is meaningless. The detached shared daemon still must not use `kill_on_drop`.

**KTD2 — Do not use `kill_on_drop` for the shared daemon.** The whole point is a long-lived shared server that survives individual MCP-client processes (per the project's launchd-deprecation / on-demand-spawn model). `kill_on_drop` would tear down the shared server when the spawning MCP process exits. The todo lists it "where lifetime allows" — here lifetime does **not** allow it. The lease plus endpoint liveness check is the correct mechanism; PID data is diagnostic only.

**KTD3 — Log to a file under the log dir.** `setup` already establishes `LSPMUX_LOG_DIR`. Redirect the spawned server's stdout/stderr to `<log_dir>/lspmux-server.log` (append) instead of `Stdio::null()`, so failures are diagnosable (R2).

**KTD4 — SEC-5 is a regression guard, not a fix.** Add a test/fixture asserting the socket dir is 0700 after `setup` runs, so the existing `chmod 700` can't silently regress. No `setup` change for SEC-5.

---

## Implementation Units

### U1. Endpoint-scoped spawn lease in `start_direct_server`

**Goal:** One coordinated spawn attempt per configured endpoint; no duplicate-spawn race.

**Requirements:** R1.

**Dependencies:** none.

**Files:**
- `mcp-server/src/bootstrap.rs` (derive a stable endpoint key; acquire/release a lease; persist PID only for diagnostics)
- `mcp-server/src/bootstrap.rs` tests

**Approach:** First probe `service_ready`. On a miss, atomically acquire an
endpoint-scoped lease in a private runtime directory. The holder spawns, waits
for readiness, records diagnostic PID data, and releases the lease. A non-holder
waits and probes again; it may reclaim only a lease proven stale by the specified
timeout/liveness rule. Make the lease location and permissions work for both Unix
and TCP endpoints.

**Execution note:** Add a failing test for the duplicate-spawn guard (two bootstraps → one server) before implementing.

**Test scenarios:**
- Covers R1: two near-simultaneous bootstraps result in exactly one spawn.
- Covers R1: a waiter observes the lease holder's ready service without spawning.
- Edge: a holder that exits before readiness is reclaimed only after the stale rule.
- Edge: malformed diagnostic PID data never grants a second lease.
- Transport: the same behavior is covered for a Unix endpoint and a TCP endpoint.

**Verification:** Repeated/concurrent bootstraps yield a single spawn attempt; diagnostic PID data reflects the ready service when available.

### U2. File logging for the spawned server

**Goal:** Diagnosable server output instead of `/dev/null`.

**Requirements:** R2.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/bootstrap.rs` (`start_direct_server`: open `<log_dir>/lspmux-server.log` append-mode for stdout/stderr instead of `Stdio::null()`)
- tests

**Approach:** Resolve the log dir (already established by `setup`/env). Open the log file in append mode and pass its fd to the child's stdout/stderr. Create the log dir if absent.

**Test scenarios:**
- Covers R2: after spawn, the configured log file exists and receives the child's output.
- Edge: log dir missing → created (or a clear error if uncreatable), never silently `/dev/null`.

**Verification:** Spawned server writes to the log file; failures are visible there.

### U3. SEC-5 regression guard (socket dir 0700)

**Goal:** Lock in the already-present `chmod 700` so it can't regress.

**Requirements:** R3, R4.

**Dependencies:** none.

**Files:**
- `mcp-server/tests/` or a shell test under the repo's shell-test path asserting `setup` leaves `LSPMUX_SOCKET_DIR` at mode 0700
- (no change to `setup` — `chmod 700` already at `setup:47`)

**Approach:** A test that runs `ensure_directories` (or `setup` in a sandbox with a temp `XDG_RUNTIME_DIR`/`TMPDIR`) and asserts the socket dir's mode is `0700`. Document that SEC-5 was already satisfied.

**Test scenarios:**
- Covers R3/R4: after directory creation, the socket dir mode is exactly 0700.
- Edge: pre-existing dir at 0755 → `setup` tightens it to 0700 (verifies `chmod` runs unconditionally, not just on create).

**Verification:** Test confirms 0700; would fail if the `chmod 700` line were removed.

---

## Scope Boundaries

In scope: SEC-4 endpoint-scoped spawn lease + file logging; a SEC-5 regression guard.

### Deferred to Follow-Up Work
- Broader daemon-lifecycle management (graceful shutdown, idle reaping) beyond duplicate-spawn prevention.

Out of scope: changing the socket transport or the daemon model; `kill_on_drop` (rejected per KTD2 — the daemon is intentionally long-lived).

---

## Risks & Dependencies

- **Lease recovery:** a crashed lease holder must not block startup forever. Specify a bounded wait and conservative stale-holder recovery; PID data is advisory because PIDs can be reused.
- **Endpoint variance:** do not rely on Unix socket binding as the backstop. TCP is a supported connection mode and has no socket-directory location for a pidfile.
- **No dependencies** on other REV todos.

---

## Verification

1. `cargo test --manifest-path mcp-server/Cargo.toml` covers lease-holder/waiter/stale recovery and a concurrent single-spawn regression for Unix and TCP endpoint keys, plus file logging (U1/U2).
2. Socket-dir 0700 regression test passes (U3) and would fail if `setup:47` were removed.
3. Manual: two rapid MCP bootstraps against clean Unix and TCP endpoint configurations leave exactly one server process per endpoint.
4. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` clean; `just shellcheck` passes if any shell changed.

---

## Sources & Research

- Origin todo: `todos/2026-05-28-server-process-hardening.md` (REV-011; consolidates SEC-4, SEC-5).
- Verified: `start_direct_server` in `mcp-server/src/bootstrap.rs` uses null stdio and drops the child handle; it supports Unix and TCP connection addresses. **SEC-5 is already fixed** by `chmod 700 "${LSPMUX_SOCKET_DIR}"` in `setup`.
- Project context: launchd plist is deprecated; the daemon is spawned on demand for its configured endpoint and multiplexes worktrees — reinforcing KTD2 (long-lived daemon, no `kill_on_drop`).
