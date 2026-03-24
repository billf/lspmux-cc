---
status: pending
priority: p1
issue_id: "REV-005"
tags: [artifacts, cargo, rust-analyzer, sccache, reuse, observability]
dependencies: ["REV-004"]
---

# Measure compiler actions and artifact reuse explicitly

A core goal of this repository is reducing redundant work across editors and agent flows, but the code currently has no way to tell when rust-analyzer or related tooling forced new compiler work versus reusing existing artifacts. That means the project cannot validate its own success condition even when a single shared rust-analyzer process exists.

## Problem Statement

The long-term operating question is not just “did we share one rust-analyzer process?” but also “did that actually reduce duplicate compile activity?” The repository should be able to answer:

- how often rust-analyzer triggered cargo/rustc work
- how often those actions reused fresh artifacts versus rebuilt work
- which client flow likely preceded the work (`Claude`, `Codex`, `Neovim`, etc.)
- whether changes such as `cargo test` plus `cargo clippy` in the same package are converging toward reuse

Today none of that is measurable from the product itself.

## Findings

- There is no code in `mcp-server/` or `plugins/` that reads cargo JSON messages, rustc wrapper output, or rust-analyzer flycheck/build status.
- The only runtime status surfaced today is `rust_server_status` (`mcp-server/src/tools.rs:571-601`), which has no artifact or build counters.
- The repository already contains an example of the data we need: `mcp-server/target/flycheck0/stdout` includes cargo JSON messages such as `compiler-artifact` with `fresh: true`, which is a direct signal for reuse versus rebuild.
- `config/lspmux.toml` passes `CARGO_HOME`, `RUSTUP_HOME`, `PATH`, `HOME`, and `USER`, but there is no instrumentation layer in the invoked toolchain path today.
- This repository is responsible for the shared rust-analyzer process per worktree, so it is the natural place to attribute rust-analyzer-induced cargo activity even if broader cache wins also involve `sccache`.
- `sccache` was visible in the local development environment during review, which reinforces the need to record both “compiler action happened” and “that action hit/missed cache layers.”

## Proposed Solutions

### Option 1: Parse rust-analyzer / flycheck outputs

**Approach:** Capture rust-analyzer status/build output (or lspmux server logs if they include it) and parse cargo JSON messages for `compiler-artifact`, `build-script-executed`, and `build-finished` events. Record `fresh` ratios and counts.

**Pros:**
- Uses existing cargo JSON semantics
- No need to intercept tool execution paths immediately
- Directly exposes `fresh` versus rebuilt signals

**Cons:**
- Depends on where rust-analyzer emits flycheck/build output
- Can be brittle if log formatting changes upstream
- Harder to correlate a build back to the initiating client/tool request

**Effort:** 1-2 days

**Risk:** Medium

---

### Option 2: Wrap cargo/rustc for rust-analyzer-owned processes (recommended)

**Approach:** Introduce a lightweight proxy binary or shell wrapper in the rust-analyzer execution environment that records every cargo/rustc invocation, parses cargo JSON when available, and emits structured events before delegating to the real toolchain.

**Pros:**
- Strongest source of truth for “forced compiler action” accounting
- Easy to correlate with worktree and process identity
- Can also sample `sccache --show-stats` deltas around build windows

**Cons:**
- Requires careful env/path management so only the intended processes are wrapped
- Slightly more invasive operationally
- Needs tests to ensure editor behavior is unchanged

**Effort:** 2-4 days

**Risk:** Medium

---

### Option 3: External process observation only

**Approach:** Observe process trees and file-system activity from the outside (for example via `ps`, `fs_usage`, or future eBPF tooling) and infer compile events.

**Pros:**
- No modifications to rust-analyzer execution path
- Useful as an independent validation channel

**Cons:**
- Weak attribution to client and workspace
- Platform-specific and likely noisy
- Hard to distinguish cache hits from genuinely skipped work

**Effort:** 2-3 days

**Risk:** High

## Recommended Action

**To be filled during triage.** Preferred direction: implement Option 2, but keep the emitted schema cargo-JSON-friendly so Option 1 can still be used as a fallback. The immediate objective is still counting rust-analyzer-induced cargo actions and reporting `fresh`/reused versus rebuilt outcomes per workspace and client kind; the current PR only finishes the attribution groundwork so those counts can be interpreted later.

## Technical Details

**Affected files:**
- `config/lspmux.toml` - possible env propagation for instrumentation wrappers
- `mcp-server/src/main.rs` - telemetry initialization and workspace labels
- `mcp-server/src/tools.rs` - expose counters/status via MCP
- `plugins/lspmux-rust-cc/bin/*` - likely place to inject wrapper binaries or environment setup
- `docs/hosts/*.md` - explain what is being measured and how to read it

**Related components:**
- `sccache` integration work outside this repository
- rust-analyzer flycheck/build jobs
- long-lived lspmux server per worktree

**Database changes (if any):**
- Migration needed? No
- New columns/tables? No

## Resources

- **Brainstorm:** `docs/brainstorms/2026-02-05-lspmux-claude-code-brainstorm.md`
- **Related todo:** `todos/2026-03-18-observability-and-client-attribution.md`
- **Local evidence:** `mcp-server/target/flycheck0/stdout` (`compiler-artifact`, `fresh: true`)
- **External docs:** cargo JSON message format and rust-analyzer status extensions

## Acceptance Criteria

- [ ] The system records when rust-analyzer triggered cargo/rustc work
- [ ] The system reports reuse vs rebuild counts (for example via cargo `fresh` signals)
- [ ] Metrics are attributable to workspace and client kind
- [ ] `rust_server_status` or a sibling tool exposes recent build/reuse statistics
- [ ] Documentation explains how this integrates with `sccache` rather than replacing it
- [ ] At least one integration test validates the emitted accounting schema on a small workspace

## Work Log

### 2026-03-18 - Initial discovery

**By:** Codex

**Actions:**
- Reviewed runtime code paths, setup/config, and local build artifacts
- Confirmed there is no existing instrumentation for cargo/rustc activity
- Noted `compiler-artifact` messages with `fresh: true` in local flycheck output as a reusable signal
- Mapped the repository’s responsibility boundary: shared rust-analyzer/worktree first, cache-system cooperation second

**Learnings:**
- “Single rust-analyzer PID” is necessary but not sufficient; reuse needs its own accounting plane
- The cheapest high-signal event appears to be cargo JSON `fresh` versus rebuilt outputs
- Client attribution should be designed first, otherwise reuse metrics will be difficult to interpret later

### 2026-03-24 - Attribution groundwork completed

**By:** Codex

**Actions:**
- Added default Claude MCP client identity injection in the wrapper so future compiler accounting can attribute activity consistently
- Added bootstrap latency accounting to the runtime snapshot so startup behavior can be correlated with future compiler reuse measurements
- Kept compiler-action capture itself deferred; no cargo/rustc interception or reuse counters were introduced in this pass

**Learnings:**
- The attribution layer now has enough structure to support compiler reuse accounting without guessing at client identity later
- The actual compiler-action capture still needs its own instrumentation path and tests

---

## Notes

- Keep the event vocabulary narrow at first: `compiler_action_started`, `compiler_action_finished`, `artifact_reused`, `artifact_rebuilt`, `sccache_stats_delta`.
- Treat direct editor `cargo test` / `cargo clippy` reuse as a separate downstream measurement, not a blocker for the initial rust-analyzer accounting work.
