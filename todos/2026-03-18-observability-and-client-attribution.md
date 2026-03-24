---
status: pending
priority: p1
issue_id: "REV-004"
tags: [observability, metrics, tracing, client-attribution, rust-analyzer, mcp]
dependencies: []
---

# Add first-class observability and client attribution

The repository currently cannot answer the operating questions that matter most for `lspmux-cc`: when the shared service is succeeding, why it failed, which client generated traffic, and whether rust-analyzer is ready or still converging. Today the code emits a small amount of startup tracing to stderr, but there is no durable per-request accounting, no per-client labels, and no machine-readable readiness signal.

## Problem Statement

`lspmux-cc` is supposed to be the layer that makes a single `rust-analyzer` instance practical across multiple tools in one worktree. That only works if operators can tell:

- whether traffic is reaching the shared instance at all
- whether failures come from bootstrap, lspmux transport, rust-analyzer health, or bad inputs
- which hosts/editors are generating successful and failing requests
- whether rust-analyzer is still indexing versus genuinely unhealthy

Without this, the project cannot prove that it is delivering the intended worktree-level sharing behavior.

## Findings

- `mcp-server/src/main.rs:88-145` initializes plain stderr tracing, but does not emit counters, histograms, request outcome labels, or structured events that are easy to aggregate.
- `mcp-server/src/tools.rs:571-601` exposes `rust_server_status`, but it only reports `running|stopped`, `workspace_root`, `server_version`, and bootstrap metadata. It does **not** surface rust-analyzer health, quiescence, last error, request counts, or failure reasons.
- `mcp-server/src/lsp_client.rs:180-195` and `532-579` log reader-loop failures and notifications, but notifications are discarded after a debug log line; there is no retained readiness state.
- `plugins/lspmux-rust-cc/.mcp.json:1-10` only passes `RUST_ANALYZER_PATH`. There is no default client identity, session identifier, or workspace label propagation into the MCP server process.
- `plugins/lspmux-rust-cc/.claude-plugin/plugin.json:5-17` likewise has no mechanism to tag LSP-originated traffic by host/editor/client kind.
- External rust-analyzer documentation exposes `experimental/serverStatus` (`health`, `quiescent`, optional `message`) and `rust-analyzer/analyzerStatus`, which are a much better basis for readiness and failure introspection than the current boolean alive check.
- External `metrics` crate documentation provides a clean path to labeled counters and histograms for request outcomes, latencies, bootstrap modes, and client labels.
- `mcp-server/src/tools.rs:290-313,370-380,420-426,480-485` convert all LSP failures to `McpError` via `map_err(|e| internal_error(...))` but never log at the conversion point. The agent sees the MCP error; stderr sees nothing. This is the "silent McpError conversion" pattern, meaning tool-level failures are invisible to operators unless `RUST_LOG=debug` is set.
- `mcp-server/src/lsp_client.rs` and `tools.rs` have zero `tracing::instrument` annotations or manual span creation. No structured fields (`request_id`, `file_path`, `duration_ms`) flow through the request lifecycle. There is no way to correlate an MCP tool call to its underlying LSP request in log output.
- `mcp-server/src/lsp_client.rs:397-410` (`ensure_file_open`) uses a content hash to skip redundant `didChange` notifications, but there is no counter or log entry for cache hits vs misses. The optimization is working but its effectiveness is unmeasurable.

## Proposed Solutions

### Option 1: Structured tracing only

**Approach:** Keep the current tracing stack, but emit JSON logs for startup, bootstrap, tool invocation, completion, and failure with fields like `tool`, `client_kind`, `workspace_root`, `outcome`, `latency_ms`, and `failure_stage`.

**Pros:**
- Smallest implementation delta
- Keeps runtime simple
- Easy to inspect locally with plain files or stderr capture

**Cons:**
- Harder to build ratios and alerts from logs alone
- No native histograms or low-cardinality counters
- Still leaves `rust_server_status` weak unless paired with readiness state

**Effort:** 4-6 hours

**Risk:** Low

---

### Option 2: Metrics-first instrumentation

**Approach:** Add `metrics` plus an exporter (Prometheus or log/snapshot based), and emit counters/histograms for tool requests, failures, bootstrap paths, and rust-analyzer readiness transitions. Use labels such as `tool`, `client_kind`, `host`, `workspace`, `outcome`, and `failure_stage`.

**Pros:**
- Directly answers success/failure rate questions
- Easy to chart reuse of service modes (`reused` vs `started_directly`)
- Better foundation for long-term SLOs

**Cons:**
- Needs an exporter story for local shell + Claude/Codex environments
- Requires care around label cardinality
- Still needs structured logs for root-cause detail

**Effort:** 1-2 days

**Risk:** Medium

---

### Option 3: Combined status model (recommended)

**Approach:** Add both structured tracing and labeled metrics, retain the last rust-analyzer readiness snapshot from `experimental/serverStatus`, and extend `rust_server_status` to include `health`, `quiescent`, `message`, per-tool counters, and recent failure summaries.

**Pros:**
- Answers both “what failed?” and “how often?”
- Gives agents a readiness signal instead of “sleep and retry”
- Supports future dashboards for client mix, error rates, and bootstrap behavior

**Cons:**
- Slightly broader change set
- Requires a small in-process state store for recent stats

**Effort:** 2-3 days

**Risk:** Medium

## Recommended Action

**To be filled during triage.** Preferred direction: continue with Option 3, but treat the client identity defaults and bootstrap latency accounting as already implemented. The remaining work is the higher-fidelity piece: decide whether a real exporter is still needed, and whether request-to-LSP span correlation is worth the added complexity.

## Technical Details

**Affected files:**
- `mcp-server/src/main.rs` - tracing initialization and request lifecycle entry point
- `mcp-server/src/tools.rs` - per-tool instrumentation and `rust_server_status` response shape
- `mcp-server/src/lsp_client.rs` - notification ingestion, readiness tracking, request timing hooks
- `plugins/lspmux-rust-cc/.mcp.json` - default MCP client labels
- `plugins/lspmux-rust-cc/.claude-plugin/plugin.json` - LSP-side client labels / metadata propagation
- `docs/hosts/*.md` - document exported metrics and expected env labels

**Related components:**
- Shared lspmux service bootstrap (`bootstrap.rs`)
- Claude session hooks, because they currently emit user-facing status strings but no machine-readable telemetry

**Database changes (if any):**
- Migration needed? No
- New columns/tables? No

## Resources

- **Brainstorm:** `docs/brainstorms/2026-02-05-lspmux-claude-code-brainstorm.md`
- **External docs:** rust-analyzer `experimental/serverStatus` and `rust-analyzer/analyzerStatus`
- **External docs:** `metrics` crate labeled counters and histograms
- **Related todo:** `todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md`
- **Related review notes:** `todos/review-2026-03-18-p2-important.md` (`AGENT-4`)

## Acceptance Criteria

- [ ] Every MCP tool call emits structured success/failure telemetry with latency
- [ ] Traffic can be grouped by client kind / host/editor
- [ ] `rust_server_status` reports rust-analyzer readiness (`health`, `quiescent`, optional message)
- [ ] Bootstrap outcomes are counted (`reused`, `started_via_manager`, `started_directly`, `skipped`, failed)
- [ ] Failure output distinguishes bootstrap failures, transport failures, rust-analyzer failures, and invalid input
- [ ] Documentation explains how to inspect the telemetry locally

## Work Log

### 2026-03-18 - Initial discovery

**By:** Codex

**Actions:**
- Audited the MCP server, Claude plugin, hooks, setup scripts, and current review notes
- Verified that runtime observability is currently limited to stderr tracing plus a minimal status tool
- Cross-checked rust-analyzer status extensions and Rust metrics crate guidance
- Identified missing client identity propagation as a blocker for traffic accounting

**Learnings:**
- The code already has a natural place for readiness state (`LspClient`) and surfaced status (`rust_server_status`)
- Client attribution is easiest if wrappers inject explicit env vars rather than inferring from process names later
- Metrics alone are not enough; detailed failures still need structured logs

### 2026-03-24 - MVP completion pass

**By:** Codex

**Actions:**
- Added default Claude MCP client identity injection in the wrapper so callers get stable attribution without manual env setup
- Threaded bootstrap latency into the in-memory telemetry snapshot and recorded it on both success and failure paths
- Clarified `rust_server_status` wording so liveness and readiness are not conflated in the summary text

**Learnings:**
- The existing telemetry shape was close to usable; the remaining work is mostly about decision quality around exporters and correlation depth
- Wrapper defaults are the lowest-friction way to make attribution consistent across Claude entry points

---

## Notes

- Keep label cardinality low: prefer `client_kind=claude_mcp|claude_lsp|codex_mcp|nvim_lsp|generic_mcp` over raw process names.
- If Prometheus feels too heavy at first, start with in-process counters + JSON log snapshots and add an exporter afterward.

### Research insights (2026-03-18 deep review)

- **Tracing patterns:** Use `#[instrument(name = "mcp.call_tool", skip(self, context), fields(tool_name = %request.name), err(level = Level::WARN))]` on boundary functions (`call_tool`, `request`, `ensure_service_running`). Skip hot helpers like `validate_file_path`, `file_uri`, `uri_to_path`. Use dot-delimited span names mirroring protocol layers: `mcp.call_tool`, `lsp.request`, `lsp.notify`, `bootstrap.service`.
- **metrics crate versions:** `metrics = "0.24"`, `metrics-exporter-prometheus = { version = "0.18", default-features = false }`, `metrics-tracing-context = "0.18"`. No HTTP port needed; use `PrometheusHandle::render()` on demand from `rust_server_status`.
- **Label cardinality rules:** NEVER use `file_path` or `request_id` as metric labels (unbounded cardinality). Use `TracingContextLayer::only_allow(["tool_name", "lsp_method", "status"])`, not `::all()`. Good labels: `tool_name` (6 values), `lsp_method` (~8), `status` (ok/error/timeout), `bootstrap_mode` (3), `service_mode` (4).
- **`experimental/serverStatus`:** Must be explicitly requested via `ClientCapabilities { experimental: Some(json!({"serverStatusNotification": true})), .. }`. Returns `{ health: ok|warning|error, quiescent: bool, message: Option<String> }`. Use `tokio::sync::watch` to retain latest value in `LspClient`; `rust_server_status` reads `status_rx.borrow()`.
- **Shell hook observability:** Use a `log_msg()` helper that writes to stderr + optional sidecar file via `$LSPMUX_LOG_DIR`. Replace `2>/dev/null || true` with `set +e; output=$(...); rc=$?; set -e; if [ $rc -ne 0 ]; then log_msg "failed (rc=$rc): $output"; fi`.
- **Anti-patterns to avoid:** Don't `#[instrument]` hot-path helpers. Don't use `tracing::info!` for request logging (use spans; they give duration and nesting). Don't forget `handle.run_upkeep()` for the Prometheus recorder. Don't bind an HTTP port for metrics in a local stdio service.
