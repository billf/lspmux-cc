---
status: pending
priority: p2
issue_id: "REV-007"
tags: [dependencies, crates, maintainability, observability, bootstrap]
dependencies: []
---

# Replace selected hand-rolled code with existing crates

Several parts of the repository are reasonable to hand-roll today, but there are a few places where mature crates would reduce maintenance cost and improve correctness. This is especially true for path discovery, metrics, and low-level Unix helpers.

## Problem Statement

The project’s differentiator is worktree-level rust-analyzer sharing and host integration, not re-implementing standard platform helpers or inventing an observability layer from scratch. Where existing crates solve the boring parts cleanly, the repository should prefer them and reserve custom code for lspmux/host-specific behavior.

## Findings

- `mcp-server/src/bootstrap.rs:73-121` and `281-296` manually resolve home/config/runtime/data-style paths for macOS and Linux.
- `mcp-server/src/bootstrap.rs:275-279` implements UID lookup by reading `UID` from the environment; existing review notes already show this is incorrect on macOS.
- `mcp-server/src/bootstrap.rs:203-205` uses an existence check for socket readiness, which can be improved with Unix file type helpers.
- `mcp-server/src/main.rs:88-95` sets up tracing, but the repository has no metrics facade or exporter.
- `mcp-server/src/lsp_client.rs` contains a custom LSP transport/client implementation. This is acceptable for now, but it is the largest remaining hand-rolled subsystem and should be periodically re-evaluated against existing libraries.
- External `directories` documentation shows `ProjectDirs`/`BaseDirs` already solve cross-platform config/data/runtime discovery.
- External `metrics` documentation shows a low-friction way to emit labeled counters and histograms, with optional Prometheus export.
- `mcp-server/src/lsp_client.rs:57-91` implements custom `file_uri()` and `uri_to_path()` functions with a hand-rolled `PATH_ENCODE_SET` for percent-encoding. The `lsp-types` crate (already in deps as v0.97) provides `lsp_types::Url::from_file_path()` and `.to_file_path()` which handle the same conversion with spec-compliant encoding. Replacing the custom functions would eliminate the `percent-encoding` dependency and ~35 lines of custom URI code. Existing tests (`file_uri_absolute_path`, `file_uri_percent_encodes_spaces`, `uri_to_path_round_trip`, `file_uri_round_trip_non_ascii`, `file_uri_rejects_relative_path`) should validate the migration.

## Proposed Solutions

### Option 1: Adopt low-risk support crates now (recommended)

**Approach:** Add:
- `directories` (or equivalent) for config/data/runtime path resolution
- `libc` or `nix` for `getuid()` and Unix file type checks
- `metrics` plus an exporter for service/tool telemetry

**Pros:**
- Targets the clearest correctness and observability wins
- Low migration risk
- Keeps the code focused on lspmux integration instead of OS trivia

**Cons:**
- Small dependency footprint increase
- Requires a few localized refactors in bootstrap and main

**Effort:** 1 day

**Risk:** Low

---

### Option 2: Keep helpers custom, add only metrics

**Approach:** Leave path/bootstrap code mostly alone, but standardize observability on `metrics` and related exporter crates.

**Pros:**
- Minimizes churn in setup-sensitive code
- Solves the most urgent missing capability (telemetry)

**Cons:**
- Leaves platform helper duplication and correctness hazards in place
- Does not address already-known UID/socket issues structurally

**Effort:** 4-6 hours

**Risk:** Low

---

### Option 3: Also replace the custom LSP transport layer

**Approach:** Evaluate `async-lsp`, `tower-lsp`, or another LSP client/runtime crate as a replacement for the hand-rolled request/response framing in `lsp_client.rs`.

**Pros:**
- Could reduce bespoke protocol code over time
- May make advanced capabilities (progress, code actions, rename, call hierarchy) easier to add

**Cons:**
- Much higher migration risk than the other replacements
- Existing transport is small enough that replacement may not pay for itself yet
- Need to verify fit for “client over child stdio to lspmux” rather than typical server use cases

**Effort:** 2-5 days

**Risk:** High

## Recommended Action

**To be filled during triage.** Preferred direction: implement Option 1, and treat Option 3 as a separate spike rather than an immediate refactor. The highest-confidence replacements are `directories`, `metrics`, and `libc`/`nix`.

## Technical Details

**Affected files:**
- `mcp-server/Cargo.toml`
- `mcp-server/src/bootstrap.rs`
- `mcp-server/src/main.rs`
- `mcp-server/src/tools.rs`
- `docs/hosts/*.md` (for telemetry documentation)

**Related components:**
- Existing review notes around bootstrap correctness and socket validation
- Observability roadmap work

**Database changes (if any):**
- Migration needed? No
- New columns/tables? No

## Resources

- **External docs:** `directories` crate (`ProjectDirs`, `BaseDirs`)
- **External docs:** `metrics` crate counters/histograms and exporter guidance
- **Related review notes:** `todos/review-2026-03-18-p1-critical.md`, `todos/review-2026-03-18-p2-important.md`
- **Related todo:** `todos/2026-03-18-observability-and-client-attribution.md`

## Acceptance Criteria

- [ ] Path discovery uses a standard crate instead of custom OS branching where practical
- [ ] UID lookup and socket/type checks rely on standard Unix APIs
- [ ] Metrics emission uses a real facade/exporter rather than ad-hoc counters
- [ ] Any deferred transport-library replacement is captured as an explicit spike, not an implicit “maybe later”
- [ ] Documentation explains which crates were adopted and why

## Work Log

### 2026-03-18 - Initial discovery

**By:** Codex

**Actions:**
- Reviewed bootstrap, LSP transport, and tracing code for custom infrastructure
- Compared current helpers against existing crate capabilities
- Identified three low-risk replacements and one high-risk exploratory replacement

**Learnings:**
- The path/bootstrap helpers are small, but still the wrong place to spend custom-maintenance budget
- Metrics support is the most valuable crate addition because it unlocks the operating questions the user cares about
- The hand-rolled LSP client is not automatically wrong; it just deserves a conscious “keep vs replace” decision

### 2026-03-24 - Current state

- `mcp-server/src/bootstrap.rs` already relies on `libc::getuid()` and Unix socket type checks, so the low-risk Unix helper replacement guidance is now mostly satisfied.
- `metrics` and `directories` are already adopted in the current codebase; exporter wiring is still a separate decision and should not be conflated with the helper-crate question.
- The remaining meaningful replacement question is the LSP transport/client layer, which should stay a separate spike rather than being pulled into bootstrap or updater hardening.

---

## Notes

- This todo is intentionally not a blanket “replace all custom code” request. The lspmux/host glue is the product and should stay custom where that improves clarity.
