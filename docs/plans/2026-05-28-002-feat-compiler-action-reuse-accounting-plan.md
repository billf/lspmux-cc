---
title: "feat: Measure compiler actions and artifact reuse (REV-005)"
status: active
date: 2026-05-28
type: feat
issue_id: REV-005
origin: todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md
---

# feat: Measure compiler actions and artifact reuse (REV-005)

## Summary

The repo's core promise is reducing redundant compile work across editors and agents, but it can't currently tell when rust-analyzer forced new cargo/rustc work versus reused fresh artifacts. This plan adds a compiler-action accounting plane: capture cargo JSON `compiler-artifact` / `build-finished` events from rust-analyzer's flycheck, count `fresh` (reused) vs rebuilt outcomes, attribute them to workspace + client kind (the REV-004 attribution groundwork already landed), and expose the counts through `rust_server_status` (or a sibling tool). Recommended direction from the todo is Option 2 (wrap cargo for rust-analyzer-owned processes), but the emitted event schema stays cargo-JSON-friendly so Option 1 (parse flycheck output) remains a fallback.

---

## Problem Frame

The project can validate "one shared rust-analyzer process per worktree" (proven by the M1 integration test) but **not** "did that reduce duplicate compile activity." There is no code in `mcp-server/` that reads cargo JSON, rustc wrapper output, or rust-analyzer flycheck/build status. The only runtime status surfaced today is `RuntimeStatus` (`mcp-server/src/bootstrap.rs:150-188`, exposed via `rust_server_status` at `mcp-server/src/tools.rs:603-700`), which carries daemon/workspace fields but no build or artifact counters.

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

**KTD1 — Capture mechanism: cargo wrapper (Option 2), schema kept Option-1-compatible.** Introduce a thin wrapper that rust-analyzer's flycheck invokes in place of `cargo` (via rust-analyzer's `check.overrideCommand` / `CARGO`-style env in the lspmux-launched toolchain environment). The wrapper streams the child's cargo JSON through unchanged (so editor behavior is identical) while tee-ing it to a parser that emits structured accounting events. Because the wrapper parses the same cargo JSON that flycheck stdout carries, Option 1 (parse flycheck output directly) stays a viable fallback without schema change.

*Rationale:* The todo marks Option 2 recommended — strongest attribution, correlates to worktree/process identity, and can sample `sccache --show-stats` deltas around a build window. Option 3 (external process observation) is rejected (weak attribution, platform-specific, high risk).

**KTD2 — Event vocabulary stays narrow.** `compiler_action_started`, `compiler_action_finished`, `artifact_reused`, `artifact_rebuilt`, `sccache_stats_delta`. No broader taxonomy in the first pass. Direct-editor `cargo test`/`cargo clippy` reuse is a separate downstream measurement, not a blocker.

**KTD3 — Counters live in the telemetry layer, exposed through status.** Accounting state belongs in `mcp-server/src/telemetry.rs` (alongside existing client-identity + latency accounting), aggregated per `(workspace, client_kind)`. `rust_server_status` gains a `build_accounting` sub-object rather than a new tool, keeping the surface small (a sibling tool is acceptable if the status response grows unwieldy — decide at implementation).

**KTD4 — Env propagation through `config/lspmux.toml`.** The wrapper path/flag must reach the rust-analyzer-owned process. `config/lspmux.toml` already has a `pass_environment` allowlist (`CARGO_HOME`, `RUSTUP_HOME`, `PATH`, `HOME`, `USER`, plus the `LSPMUX_CLIENT_*` vars); add the wrapper's control var(s) there so only the intended processes are wrapped.

---

## Implementation Units

### U1. Define the accounting event schema and counters in telemetry

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

### U2. Cargo-JSON parser that maps cargo messages to accounting events

**Goal:** Turn a stream of cargo JSON lines into U1 events.

**Requirements:** R1, R2.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/build_accounting.rs` (new; parser) and `mcp-server/src/lib.rs` (`pub mod build_accounting;`)
- `mcp-server/src/build_accounting.rs` tests, using captured fixtures from `mcp-server/target/flycheck0/stdout`

**Approach:** Parse line-delimited cargo JSON. Recognize `compiler-artifact` (read `fresh`), `build-script-executed`, and `build-finished`. Emit U1 events. Ignore unrecognized message kinds. Robust to partial/non-JSON lines (flycheck interleaves human output).

**Patterns to follow:** Existing line-framed parsing in `mcp-server/src/lsp_client.rs` (`send_message`/read loop) for streaming-robustness style.

**Test scenarios:**
- Covers R2: a fixture line with `"reason":"compiler-artifact","fresh":true` produces exactly one `artifact_reused`.
- Happy path: a real captured flycheck fixture produces the expected reused/rebuilt totals.
- Edge: interleaved non-JSON / human-readable lines are skipped without error.
- Edge: truncated final line does not panic.
- Error path: malformed JSON object is counted as skipped, not fatal.

**Verification:** Parser turns the committed fixture into the documented counts.

### U3. Cargo wrapper that tees cargo JSON to the parser

**Goal:** A wrapper binary/shim that rust-analyzer flycheck invokes, passing cargo output through unchanged while feeding U2.

**Requirements:** R1, R3.

**Dependencies:** U2.

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

### U4. Expose build/reuse stats through `rust_server_status`

**Goal:** Surface recent accounting to agents.

**Requirements:** R4.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/bootstrap.rs` (extend `RuntimeStatus` with a `build_accounting` sub-object) — or `mcp-server/src/tools.rs` if a sibling tool is chosen
- `mcp-server/src/tools.rs` (`rust_server_status` response shaping at ~603-700)
- `mcp-server/src/tools.rs` / `tests/` response-shape test

**Approach:** Add a nested `build_accounting` field (reused, rebuilt, last-window timestamps, per-client breakdown) to the status response. Keep it additive so existing `RuntimeStatus` consumers are unaffected.

**Test scenarios:**
- Covers R4: status response includes `build_accounting` with reused/rebuilt counts after events are recorded.
- Edge: no events yet → field present with zeroed counts (not absent), so agents can rely on the shape.
- Integration: recording events via U1 then calling status reflects them.

**Verification:** `rust_server_status` JSON carries the counters; schema documented.

### U5. sccache integration doc + accounting schema doc

**Goal:** Explain what is measured, how to read it, and how it cooperates with `sccache`.

**Requirements:** R5.

**Dependencies:** U4.

**Files:**
- `docs/hosts/claude-code.md` (or a new `docs/observability.md`) — explain the counters and reading them
- Brief note on `sccache_stats_delta` sampling and that this accounting attributes rust-analyzer-induced cargo activity, not replacing cache layers

**Test expectation:** none — documentation only.

**Verification:** Doc explains the event vocabulary, the status field, and the sccache relationship.

### U6. Integration test validating the accounting schema on a small workspace

**Goal:** End-to-end proof on a minimal cargo workspace.

**Requirements:** R6.

**Dependencies:** U2, U3, U4.

**Files:**
- `mcp-server/tests/build_accounting.rs` (new integration test, `#[ignore]`-gated like existing integration tests if it needs real cargo)

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

Out of scope: replacing or configuring `sccache` itself (lives outside this repo); eBPF/process-tree observation (Option 3, rejected).

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
4. `rust_server_status` returns `build_accounting` with non-trivial counts after a build window.
5. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` stays clean.

---

## Sources & Research

- Origin todo: `todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md` (REV-005).
- Verified locations: `RuntimeStatus` at `mcp-server/src/bootstrap.rs:150-188`; `rust_server_status` at `mcp-server/src/tools.rs:603-700`; client identity + latency accounting in `mcp-server/src/telemetry.rs`; `pass_environment` in `config/lspmux.toml`.
- Local evidence: `mcp-server/target/flycheck0/stdout` (`compiler-artifact`, `fresh: true`).
- Related (done): REV-004 attribution groundwork; `docs/brainstorms/archive/2026-02-05-lspmux-claude-code-brainstorm.md`.
- Note: ARCH-1 (lib/bin split, `docs/brainstorms/2026-05-04-mcp-server-workspace-split-requirements.md`) is not done; `build_accounting.rs` and the parser go in the existing library crate (`lib.rs`), reachable by both the binary and tests.
