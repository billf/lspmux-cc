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

The direct-spawn bootstrap path (`start_direct_server()` in `mcp-server/src/bootstrap.rs:496-509`) spawns `lspmux server` with null stdio and immediately drops the child handle — no PID tracking, no pidfile, no `kill_on_drop`. Concurrent invocations can race multiple servers onto one socket, and orphans accumulate. This plan adds pidfile-based single-instance guarding and file logging for the spawned server. **SEC-5 (socket dir 0700) is already fixed** — `setup:47` already runs `chmod 700 "${LSPMUX_SOCKET_DIR}"` — so this plan is scoped to SEC-4 only and records SEC-5 as already satisfied with a regression test.

---

## Problem Frame

- **SEC-4 (open):** `start_direct_server()` (`mcp-server/src/bootstrap.rs:496-509`) builds `lspmux server --config <path>`, sets `stdout`/`stderr` to `Stdio::null()`, calls `.spawn()`, and drops the returned child. There is no PID stored, no pidfile checked before spawning, and no `kill_on_drop`. Concurrent bootstraps can spawn duplicate servers contending for one socket; orphaned servers accumulate over time. Spawned-server output is discarded (`/dev/null`), so failures are undiagnosable.
- **SEC-5 (already resolved):** the review flagged the socket directory at default 0755 (world-accessible socket under `/tmp/lspmux/`). Verification shows `setup` already creates the dirs and runs `chmod 700 "${LSPMUX_SOCKET_DIR}"` (`setup:46-47`). No code change needed; add a regression guard so it can't silently regress.

Socket dir precedence (verified, `setup:13-23`): `XDG_RUNTIME_DIR` → `TMPDIR` → `/tmp`, with the lspmux dir under `RUNTIME_BASE`.

---

## Requirements

- **R1.** Direct-spawn writes and checks a pidfile; no duplicate servers on one socket.
- **R2.** The spawned server logs to a file, not `/dev/null`.
- **R3.** Socket directory is created mode 0700. *(Already satisfied at `setup:47`; this plan adds a regression guard, not a new fix.)*
- **R4.** Regression tests/fixtures cover the pidfile and permission invariants.

---

## Key Technical Decisions

**KTD1 — Pidfile beside the socket, checked before spawn.** Write `<socket_dir>/lspmux-server.pid` after a successful spawn. Before spawning, read the pidfile and check liveness (signal-0 / `kill(pid, 0)` semantics). If a live server owns the socket, do not spawn; reuse it. Stale pidfile (process gone) → overwrite and spawn. This is the minimal correct guard against the duplicate-server race.

*Rationale:* The bootstrap already resolves the socket path; co-locating the pidfile keeps lifetime tied to the socket. A pidfile is simpler and more portable across the detach boundary than holding the child handle (the server is intentionally long-lived and outlives the MCP process, so `kill_on_drop` is the wrong default — see KTD2).

**KTD2 — Do not use `kill_on_drop` for the shared daemon.** The whole point is a long-lived shared server that survives individual MCP-client processes (per the project's launchd-deprecation / on-demand-spawn model). `kill_on_drop` would tear down the shared server when the spawning MCP process exits. The todo lists it "where lifetime allows" — here lifetime does **not** allow it. Pidfile + liveness check is the correct mechanism.

**KTD3 — Log to a file under the log dir.** `setup` already establishes `LSPMUX_LOG_DIR`. Redirect the spawned server's stdout/stderr to `<log_dir>/lspmux-server.log` (append) instead of `Stdio::null()`, so failures are diagnosable (R2).

**KTD4 — SEC-5 is a regression guard, not a fix.** Add a test/fixture asserting the socket dir is 0700 after `setup` runs, so the existing `chmod 700` can't silently regress. No `setup` change for SEC-5.

---

## Implementation Units

### U1. Pidfile write + pre-spawn liveness check in `start_direct_server`

**Goal:** One server per socket; no duplicate-spawn race; no orphan accumulation.

**Requirements:** R1.

**Dependencies:** none.

**Files:**
- `mcp-server/src/bootstrap.rs` (`start_direct_server` ~496-509; add pidfile path derivation, pre-spawn check, post-spawn write)
- `mcp-server/src/bootstrap.rs` tests

**Approach:** Derive `<socket_dir>/lspmux-server.pid`. Before spawning: if the pidfile exists and names a live process, skip spawn (reuse). Otherwise spawn, capture the child PID, write it to the pidfile (atomically: write temp + rename). Treat a pidfile naming a dead PID as stale and overwrite. Use the resolved socket dir already known to the bootstrap.

**Execution note:** Add a failing test for the duplicate-spawn guard (two bootstraps → one server) before implementing.

**Test scenarios:**
- Covers R1: with a live pidfile present, `start_direct_server` does not spawn a second process.
- Covers R1: stale pidfile (dead PID) → spawns and overwrites the pidfile.
- Edge: missing pidfile → spawns and writes it.
- Edge: pidfile with garbage contents → treated as stale, not a panic.
- Concurrency: two near-simultaneous bootstraps result in exactly one server (atomic write/rename + check ordering).

**Verification:** Repeated/concurrent bootstraps yield a single server; pidfile reflects the live PID.

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

In scope: SEC-4 pidfile + single-instance guard + file logging; a SEC-5 regression guard.

### Deferred to Follow-Up Work
- Broader daemon-lifecycle management (graceful shutdown, idle reaping) beyond duplicate-spawn prevention.

Out of scope: changing the socket transport or the daemon model; `kill_on_drop` (rejected per KTD2 — the daemon is intentionally long-lived).

---

## Risks & Dependencies

- **Pidfile races:** naive check-then-spawn has a TOCTOU window. Mitigated by atomic temp-write + rename and accepting that the liveness check plus single socket binding (the server will fail to bind a busy socket) is the backstop. Note: rust-analyzer/lspmux itself failing to bind a taken socket is the ultimate guard; the pidfile prevents the common case and orphan accumulation.
- **PID reuse:** a recycled PID could look "live." Low risk for a short-lived check; acceptable given the socket-bind backstop.
- **No dependencies** on other REV todos.

---

## Verification

1. `cargo test --manifest-path mcp-server/Cargo.toml` covers pidfile present/stale/missing/garbage and the concurrency guard (U1), file-logging (U2).
2. Socket-dir 0700 regression test passes (U3) and would fail if `setup:47` were removed.
3. Manual: two rapid MCP bootstraps against a clean socket dir leave exactly one `lspmux server` process and one pidfile.
4. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` clean; `just shellcheck` passes if any shell changed.

---

## Sources & Research

- Origin todo: `todos/2026-05-28-server-process-hardening.md` (REV-011; consolidates SEC-4, SEC-5).
- Verified: `start_direct_server` `mcp-server/src/bootstrap.rs:496-509` (null stdio, dropped handle, no pidfile); **SEC-5 already fixed** — `chmod 700 "${LSPMUX_SOCKET_DIR}"` at `setup:47`; socket-dir precedence `setup:13-23`; `RuntimeStatus`/runtime assembly `bootstrap.rs:150-188,361-388`.
- Project context: launchd plist deprecated; daemon spawned on-demand per worktree (memory `project_launchd_service`) — reinforces KTD2 (long-lived daemon, no `kill_on_drop`).
