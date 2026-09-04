---
title: "feat: Measure compiler actions and artifact reuse (REV-005)"
status: partial
date: 2026-05-28
audited: 2026-09-04
type: feat
issue_id: REV-005
origin: todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md
---

# feat: Measure compiler actions and artifact reuse (REV-005)

## Summary

Partially delivered. `rust_server_status` already exposes a best-effort
`compiler_accounting` snapshot by parsing the most recently modified
`target/flycheck*/stdout` cargo-JSON file in the selected workspace. It reports
artifact, fresh, rebuilt, build-script, build-finished, and parse-error counts.

That is useful local observability, but it is not the original attribution
system: it does not establish action windows, distinguish clients, or guarantee
that the chosen flycheck file belongs to the current MCP session. The original
wrapper proposal also leaves its event transport and rust-analyzer configuration
contract unresolved. Treat that as a separate design-and-delivery effort rather
than expanding this low-risk telemetry slice into cross-process plumbing.

## Audit and composition boundary

| Slice | State | Evidence |
|---|---|---|
| Parse cargo JSON artifact freshness | delivered | `TelemetryState::refresh_compiler_accounting` parses `target/flycheck*/stdout`. |
| Expose a status snapshot | delivered | `rust_server_status` returns `compiler_accounting`. |
| Action start/finish lifecycle | not delivered | No wrapper or event sink exists. |
| Workspace and client-kind attribution | not delivered | The snapshot is selected only by workspace file recency. |
| `sccache` deltas and end-to-end proof | not delivered | No sampling or dedicated integration test exists. |

Keep the delivered scanner as a read-only, best-effort metric. A future precise
accounting plan must first choose and test a durable event boundary (for example,
a daemon-owned event endpoint or a versioned append-only stream), then separately
wire a cargo wrapper. It must not use a mutable `target/` scan as evidence of
per-client attribution.

---

## Problem Frame

The project can validate "one shared rust-analyzer process per worktree" (proven by the M1 integration test), but it cannot yet show that this reduced duplicate compiler activity for a particular client. The delivered `compiler_accounting` scanner reads local cargo JSON, but it has no action lifecycle, client attribution, or session ownership.

Local evidence that the needed signal exists: `mcp-server/target/flycheck0/stdout` contains cargo JSON messages such as `compiler-artifact` with `"fresh": true` — a direct reuse-vs-rebuild signal.

REV-004 (done) already injects client identity (`LSPMUX_CLIENT_KIND` / `_HOST` / `_SESSION_ID`, read in `mcp-server/src/telemetry.rs`) and bootstrap latency accounting, so attribution context exists to interpret reuse counts.

This todo's prior pass deliberately deferred the actual compiler-action capture; that capture and its tests are this plan's work.

---

## Requirements

- **R1.** The system records when rust-analyzer triggered cargo/rustc work (a `compiler_action_started` / `compiler_action_finished` pair, or equivalent count).
- **R2.** The system reports reuse vs rebuild counts derived from cargo `fresh` signals (`artifact_reused` vs `artifact_rebuilt`).
- **R3.** Counts are attributable to workspace and client kind, reusing the REV-004 attribution fields.
- **R4.** `rust_server_status` (or a sibling tool) exposes recent build/reuse statistics.
- **R5.** Documentation explains how this integrates with `sccache` (cooperates, does not replace it).
- **R6.** At least one integration test validates the emitted accounting schema on a small workspace.

---

## Key Technical Decisions

**KTD1 — Keep the current scanner separate from precise event ingestion.** The
delivered scanner reads cargo JSON after the fact and must remain labelled
best-effort. Do not add a cargo wrapper until a separate plan defines a durable
event transport, ownership/authentication, retention, and the exact
rust-analyzer configuration contract. A wrapper without that boundary can alter
flycheck behavior while still failing to attribute its output correctly.

**KTD2 — Event vocabulary stays narrow.** `compiler_action_started`, `compiler_action_finished`, `artifact_reused`, `artifact_rebuilt`, `sccache_stats_delta`. No broader taxonomy in the first pass. Direct-editor `cargo test`/`cargo clippy` reuse is a separate downstream measurement, not a blocker.

**KTD3 — Precise counters belong in telemetry and extend `compiler_accounting`.** Keep the existing status field rather than introducing a second name. A future attributable implementation may aggregate by `(workspace, client_kind)` only after its event boundary establishes those values; a sibling tool is unnecessary unless the status payload becomes unwieldy.

**KTD4 — Env propagation through `config/lspmux.toml`.** The wrapper path/flag must reach the rust-analyzer-owned process. `config/lspmux.toml` already has a `pass_environment` allowlist (`CARGO_HOME`, `RUSTUP_HOME`, `PATH`, `HOME`, `USER`, plus the `LSPMUX_CLIENT_*` vars); add the wrapper's control var(s) there so only the intended processes are wrapped.

---

## Implementation Units

### U1. Define an attributable event schema and counters in telemetry — deferred

**Goal:** A typed, cargo-JSON-friendly event vocabulary and per-`(workspace, client_kind)` counters.

**Requirements:** R1, R2, R3.

**Dependencies:** none.

**Files:**
- `mcp-server/src/telemetry.rs` (add accounting event enum + counter aggregation)
- `mcp-server/src/telemetry.rs` tests (unit tests for counter math)

**Approach:** Model the five KTD2 events as a serde-serializable enum. Maintain counters keyed by `(workspace_root, client_kind)` reusing fields already resolved in `telemetry.rs`. Expose an accumulate method and a snapshot method. Keep `fresh: true` → `artifact_reused`, otherwise `artifact_rebuilt`.

**Patterns to follow:** Existing `TelemetryState::from_env` and bootstrap-latency accounting in `mcp-server/src/telemetry.rs`.

**Test scenarios:**
- Happy path: feeding a sequence of N `compiler-artifact` events with mixed `fresh` flags yields correct reused/rebuilt counts.
- Edge: zero events → all counters zero, snapshot is well-formed.
- Edge: events for two distinct client kinds aggregate into separate buckets.
- Covers R2/R3: a `build-finished` event closes an action window and attributes it to the active workspace+client.

**Verification:** Counter math unit tests pass; snapshot serializes to the documented schema.

### U2. Cargo-JSON parser that maps cargo messages to accounting events — partially delivered

**Goal:** Turn a stream of cargo JSON lines into U1 events.

**Requirements:** R1, R2.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/telemetry.rs` (the delivered best-effort scanner and any future parser boundary)
- committed test fixtures under `mcp-server/tests/fixtures/` if a parser is extracted

**Approach:** Parse line-delimited cargo JSON. Recognize `compiler-artifact` (read `fresh`), `build-script-executed`, and `build-finished`. Emit U1 events. Ignore unrecognized message kinds. Robust to partial/non-JSON lines (flycheck interleaves human output).

**Patterns to follow:** Existing line-framed parsing in `mcp-server/src/lsp_client.rs` (`send_message`/read loop) for streaming-robustness style.

**Test scenarios:**
- Covers R2: a fixture line with `"reason":"compiler-artifact","fresh":true` produces exactly one `artifact_reused`.
- Happy path: a real captured flycheck fixture produces the expected reused/rebuilt totals.
- Edge: interleaved non-JSON / human-readable lines are skipped without error.
- Edge: truncated final line does not panic.
- Error path: malformed JSON object is counted as skipped, not fatal.

**Verification:** Parser turns the committed fixture into the documented counts.

### U3. Cargo wrapper that tees cargo JSON to the parser — split out

**Goal:** A wrapper binary/shim that rust-analyzer flycheck invokes, passing cargo output through unchanged while feeding U2.

**Requirements:** R1, R3.

**Dependencies:** A separately approved event-ingestion design; do not start
from this plan.

**Files:**
- `mcp-server/src/bin/cargo-accounting-wrapper.rs` (new bin) OR a shell shim under `bin/` — decide at implementation (see Open Questions)
- `config/lspmux.toml` (add wrapper control env var to `pass_environment`)
- `bin/` (env/path setup so only rust-analyzer-owned processes are wrapped)

**Approach:** `exec` the real cargo with identical args, tee-ing stdout through the U2 parser to an accounting sink (IPC to the MCP server, or an append-only event file the server tails — decide at implementation). Must be transparent: exit code, stdout, stderr identical to bare cargo. Guard with an env flag so editor `cargo` invocations outside rust-analyzer are never wrapped.

**Execution note:** Add a transparency test (wrapper output byte-identical to cargo) before wiring it into the toolchain path.

**Test scenarios:**
- Covers R1: invoking the wrapper around a fake cargo that emits known JSON records the expected events.
- Critical (editor-unchanged): wrapper forwards stdout/stderr/exit-code identically to the underlying command.
- Edge: underlying cargo fails (nonzero exit) → wrapper still forwards and records a finished action.
- Edge: control env flag absent → wrapper no-ops / is not on the path (no accidental wrapping).

**Verification:** Transparency test green; wrapped fake-cargo run produces correct counts attributed to the active workspace/client.

### U4. Expose build/reuse stats through `rust_server_status` — delivered for the best-effort snapshot

**Goal:** Surface recent accounting to agents.

**Requirements:** R4.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/tools.rs` (extend the existing `compiler_accounting` response object if attributable counters are delivered)
- `mcp-server/src/tools.rs` (`rust_server_status` response shaping at ~603-700)
- `mcp-server/src/tools.rs` / `tests/` response-shape test

**Approach:** Extend `compiler_accounting` with reused, rebuilt, action-window timestamps, and per-client breakdown only after those values have a durable event source. Keep additions backward-compatible.

**Test scenarios:**
- Covers R4: status response includes attributable reused/rebuilt counts after events are recorded.
- Edge: no events yet → field present with zeroed counts (not absent), so agents can rely on the shape.
- Integration: recording events via U1 then calling status reflects them.

**Verification:** `rust_server_status` JSON carries the counters; schema documented.

### U5. sccache integration doc + accounting schema doc — deferred with U3

**Goal:** Explain what is measured, how to read it, and how it cooperates with `sccache`.

**Requirements:** R5.

**Dependencies:** U4.

**Files:**
- `docs/hosts/claude-code.md` (or a new `docs/observability.md`) — explain the counters and reading them
- Brief note on `sccache_stats_delta` sampling and that this accounting attributes rust-analyzer-induced cargo activity, not replacing cache layers

**Test expectation:** none — documentation only.

**Verification:** Doc explains the event vocabulary, the status field, and the sccache relationship.

### U6. Integration test validating the accounting schema on a small workspace — split by metric level

**Goal:** End-to-end proof on a minimal cargo workspace.

**Requirements:** R6.

**Dependencies:** U2, U3, U4.

**Files:**
- `mcp-server/tests/compiler_accounting.rs` (new integration test, `#[ignore]`-gated if it needs real cargo)

**Approach:** Drive a tiny fixture crate through the wrapper (or feed captured cargo JSON to the parser for a hermetic variant), then assert the status response reports the expected reused/rebuilt split. Mirror the `#[ignore]` + binary-availability pattern in `mcp-server/tests/integration.rs`.

**Test scenarios:**
- Covers R6: first build → all rebuilt; immediate re-run → all reused; status reflects both windows.
- Attribution: events carry the test's client kind/workspace.

**Verification:** Test passes (or is correctly `#[ignore]`-gated with documented run instructions).

---

## Scope Boundaries

In scope: rust-analyzer-induced cargo action accounting (counts, reuse/rebuild, attribution, status surface, docs, one integration test).

### Deferred to Follow-Up Work
- Direct-editor `cargo test` / `cargo clippy` reuse measurement (explicitly a separate downstream measurement per the todo).
- Cache hit/miss correlation beyond `sccache_stats_delta` sampling.

Out of scope: replacing or configuring `sccache` itself (lives outside this repo); eBPF/process-tree observation; and cross-process wrapper delivery without its own event-ingestion contract.

---

## Risks & Dependencies

- **Wrapper scoping risk:** wrapping the wrong cargo invocations would distort counts and could affect editor behavior. Mitigated by the control env flag (KTD4) and the transparency test (U3).
- **Flycheck output dependency:** if Option-1 fallback is ever used, log-format drift upstream is a risk; KTD1 keeps the schema cargo-JSON-based to minimize this.
- **Sink/IPC design is an execution-time unknown** (file-tail vs in-process channel) — see Open Questions.
- Depends on REV-004 attribution fields (done).

---

## Open Questions (resolve during implementation)

- Wrapper as a Rust `bin` vs a shell shim under `bin/` — Rust bin gives portable parsing; shell shim is lighter. Decide when wiring the toolchain path.
- Accounting sink transport: append-only event file the server tails, vs in-process channel if the wrapper can share the server's address space (it can't if it's a separate process) — likely a small event file under the socket/runtime dir.
- Whether to extend `RuntimeStatus` (additive field) or add a sibling `rust_build_stats` tool if the status payload grows too large.

---

## Verification

1. Unit: `cargo test --manifest-path mcp-server/Cargo.toml` covers U1 counter math and U2 parser fixtures.
2. Transparency: U3 wrapper output is byte-identical to bare cargo on a sample command.
3. Integration: U6 small-workspace test shows rebuilt-then-reused across two builds, attributed to workspace+client.
4. `rust_server_status` returns attributable `compiler_accounting` counts after a build window.
5. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` stays clean.

---

## Sources & Research

- Origin todo: `todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md` (REV-005).
- Verified locations: `RuntimeStatus` at `mcp-server/src/bootstrap.rs:150-188`; `rust_server_status` at `mcp-server/src/tools.rs:603-700`; client identity + latency accounting in `mcp-server/src/telemetry.rs`; `pass_environment` in `config/lspmux.toml`.
- Local evidence: `mcp-server/target/flycheck0/stdout` (`compiler-artifact`, `fresh: true`).
- Related (done): REV-004 attribution groundwork; `docs/brainstorms/archive/2026-02-05-lspmux-claude-code-brainstorm.md`.
- Audit note: the crate already has a library target and external `LspClient`
  integration coverage. Exposing the MCP tool router from the library remains a
  separate design choice, not a prerequisite for parser or telemetry tests.
