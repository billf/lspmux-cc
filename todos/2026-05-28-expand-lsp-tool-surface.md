---
status: planned
priority: p2
issue_id: "REV-010"
tags: [mcp, tools, lsp, rust-analyzer, agent-native]
dependencies: []
plan: "docs/plans/2026-05-28-004-feat-expand-lsp-tool-surface-plan.md"
---

# Expand the MCP LSP tool surface

Consolidates the agent-native tool gaps from the 2026-03-18 review (AGENT-1/2/3
in `review-2026-03-18-p1-critical.md`, AGENT-4..8 in
`review-2026-03-18-p2-important.md`), now archived under `todos/archive/`.

## Problem Statement

`mcp-server/src/tools.rs` exposes six tools (`rust_diagnostics`, `rust_hover`,
`rust_goto_definition`, `rust_find_references`, `rust_workspace_symbol`,
`rust_server_status`). Several high-value rust-analyzer capabilities a human
editor uses daily are not reachable by an agent, forcing manual edits, full-file
reads, and "wait and retry" guessing.

## Findings (from review)

- **AGENT-1 (code actions):** no `textDocument/codeAction` wrapper. When
  `rust_diagnostics` reports a fixable error (missing import, unused var), the
  agent guesses instead of applying RA's quick fix.
- **AGENT-2 (rename):** no `textDocument/rename` wrapper. Cross-codebase renames
  require manual `find_references` plus per-file edits.
- **AGENT-3 (document symbols):** no `textDocument/documentSymbol` wrapper. No
  way to outline a single file without reading and parsing it.
- **AGENT-4 (readiness / wait-for-ready):** instructions say "wait a few seconds
  and retry." Ingest RA `experimental/serverStatus` / `$/progress` and expose
  `{ indexing: bool, quiescent: bool, health }` through `rust_server_status`.
- **AGENT-5 (call hierarchy):** no `callHierarchy/incomingCalls` /
  `outgoingCalls`. Outgoing calls can't be traced at all.
- **AGENT-6 (go-to-implementation):** no `textDocument/implementation` wrapper
  (distinct from references).
- **AGENT-7 (client capabilities):** the client advertises
  `ClientCapabilities::default()` (`mcp-server/src/lsp_client.rs:218`). New tools
  must expand advertised capabilities or RA may withhold the features. This is a
  prerequisite for the tools above, not a standalone tool.
- **AGENT-8 (expand-macro):** no `rust-analyzer/expandMacro` wrapper; high value
  for macro-heavy code.

## Proposed approach

Add the tools incrementally, each wrapping the corresponding LSP request and
returning edits/locations as data without applying them. Expand
`ClientCapabilities` (AGENT-7) first so RA offers the capabilities the new tools
depend on. Suggested order by value: code actions, document symbols, rename,
readiness signal, then call hierarchy / go-to-implementation / expand-macro.

This work is easier once `tools.rs` is in the library crate (see ARCH-1 in
`docs/brainstorms/2026-05-04-mcp-server-workspace-split-requirements.md`),
enabling integration tests for tool dispatch and response shaping.

## Acceptance Criteria

- [ ] `ClientCapabilities` advertises the features the new tools require (AGENT-7)
- [ ] `rust_code_actions` returns available actions with their edits (AGENT-1)
- [ ] `rust_rename` returns the workspace edit without applying it (AGENT-2)
- [ ] `rust_document_symbols` returns the file's symbol tree (AGENT-3)
- [ ] `rust_server_status` reports indexing/readiness state (AGENT-4)
- [ ] call-hierarchy and go-to-implementation tools added (AGENT-5/6)
- [ ] `rust_expand_macro` added (AGENT-8)
- [ ] Each new tool has parameter-validation and response-shaping tests

## Work Log

### 2026-05-28 - Promoted from review snapshots

**By:** Claude Code (todos/brainstorms consolidation)

**Actions:**
- Consolidated AGENT-1..8 from the archived 2026-03-18 review files into one
  tracked tool-surface todo.

**Learnings:**
- AGENT-7 (client capabilities) gates the rest and should land first.
