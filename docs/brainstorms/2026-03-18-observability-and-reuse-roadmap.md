# lspmux-cc: observability and artifact-reuse roadmap

**Date:** 2026-03-18
**Status:** Brainstorm
**Context:** Follow-up to `docs/brainstorms/2026-02-05-lspmux-claude-code-brainstorm.md`

## What changed in my understanding during review

The original brainstorm correctly framed the primary problem as “too many `rust-analyzer` instances per worktree.” After reviewing the repository, the next-order problem is clearer:

1. we still need the **single shared rust-analyzer per worktree** invariant
2. we also need to **observe** whether that invariant is actually reducing duplicate compiler work
3. we need enough attribution to say **which client** caused traffic and whether that client is succeeding or failing

In other words, a single PID is necessary, but it is not yet sufficient evidence that the system is doing the right thing.

## What this repository should know at runtime

For each worktree, the system should be able to answer all of the following without log archaeology:

- Is there exactly one `rust-analyzer` behind lspmux for this worktree?
- Was the shared service reused, started via launchd/systemd, or started directly?
- Is rust-analyzer healthy, partially degraded, or still indexing?
- Which clients are sending traffic? (`claude_lsp`, `claude_mcp`, `codex_mcp`, `nvim_lsp`, `generic_mcp`, etc.)
- What is the per-client success/failure rate?
- What are the common failure classes? (bootstrap, transport, indexing/not-ready, invalid input, rust-analyzer internal error)
- How often did a request force compiler activity?
- Of those compiler activities, how many reused fresh artifacts versus rebuilt work?

## Readiness model: stop guessing, start reporting

Right now the user-facing guidance is effectively “wait 2-3 seconds and retry.” That is a placeholder, not a readiness model.

rust-analyzer already exposes better primitives:

- `experimental/serverStatus`
  - `health = ok | warning | error`
  - `quiescent = true | false`
  - optional human-readable `message`
- `rust-analyzer/analyzerStatus`
  - useful for deep debugging and dependency/context inspection

The MCP server should ingest and retain that state, then expose it through `rust_server_status` and telemetry.

## Client attribution model

Attribution should be explicit, not inferred after the fact.

### Proposed env contract

Each wrapper or host entry point should inject stable labels such as:

- `LSPMUX_CLIENT_KIND=claude_lsp|claude_mcp|codex_mcp|nvim_lsp|generic_mcp`
- `LSPMUX_CLIENT_HOST=claude|codex|nvim|generic`
- `LSPMUX_SESSION_ID=<uuid-or-host-session-id>`
- `WORKSPACE_ROOT=<absolute worktree root>`

This is preferable to trying to reverse-engineer client type from process trees or argv later.

## Suggested telemetry surface

### Counters

- `lspmux_cc_tool_requests_total{tool,client_kind,outcome}`
- `lspmux_cc_bootstrap_total{service_mode}`
- `lspmux_cc_bootstrap_failures_total{stage}`
- `lspmux_cc_ra_status_transitions_total{health}`
- `lspmux_cc_compiler_actions_total{workspace,client_kind}`
- `lspmux_cc_artifact_reuse_total{workspace,client_kind,result=fresh|rebuilt}`

### Histograms

- `lspmux_cc_tool_latency_seconds{tool,client_kind}`
- `lspmux_cc_bootstrap_latency_seconds{service_mode}`
- `lspmux_cc_ra_quiescence_wait_seconds{workspace}`

### Structured logs

Emit one structured event for:

- bootstrap attempt/result
- MCP tool start/result
- LSP transport failure
- rust-analyzer readiness transition
- compiler-action observation

## Where artifact-reuse data can come from

There are three practical sources, in order of increasing control:

### 1. rust-analyzer / flycheck output

Cargo JSON already contains high-value signals like `compiler-artifact` and `fresh: true`. This is the quickest proof-of-concept path if that output can be captured reliably.

### 2. wrapper/proxy around cargo or rustc

A lightweight wrapper placed in the toolchain path for rust-analyzer-owned work can emit structured accounting events and then delegate to the real binary. This is probably the strongest medium-term design because it improves both attribution and data quality.

### 3. external process observation

Useful as a validation backstop, but weak as the main source of truth because attribution and cache classification are difficult.

## Crates worth adopting

These look like good fits for the next phase:

- `metrics` (+ exporter): labeled counters/histograms
- `metrics-tracing-context`: attach tracing context as metric labels where appropriate
- `directories`: cross-platform config/data/runtime path resolution
- `libc` or `nix`: correct UID lookup and Unix file-type helpers

A larger question is whether to replace the hand-rolled LSP client transport with an existing library such as `async-lsp` or `tower-lsp`-adjacent infrastructure. My current view is:

- **not urgent right now**
- worth a spike once observability is in place
- only justified if it makes advanced tooling and notification handling meaningfully simpler

## What “good” looks like for the long-term goal

Given the user’s target scenario:

- file open in two editors
- `cargo test` from one editor
- `cargo clippy` against the containing package
- `sccache` work happening in parallel elsewhere

Success should look like this:

1. one shared `rust-analyzer` instance per worktree
2. a visible, queryable record of which clients touched that worktree
3. a readiness signal showing whether RA was still converging when requests were made
4. a measurable compiler-action ledger showing fresh/reused versus rebuilt work
5. a clear story about where this repository stops and `sccache` / broader cargo coordination begins

## Near-term roadmap

### Phase 1: observability foundation

- add explicit client identity propagation
- add structured logs and metrics
- ingest rust-analyzer readiness notifications
- extend `rust_server_status`

### Phase 2: compiler-action accounting

- capture cargo JSON or proxy cargo/rustc for rust-analyzer-owned work
- emit reuse vs rebuild counters
- correlate with client kind and workspace

### Phase 3: validation and tuning

- add integration tests for telemetry schemas
- validate one-RA-per-worktree invariant across multiple clients
- build small dashboards or debug commands for local inspection

## Related todos

- `todos/2026-03-18-observability-and-client-attribution.md`
- `todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md`
- `todos/2026-03-18-linux-hook-bootstrap-parity.md`
- `todos/2026-03-18-crate-replacement-opportunities.md`
