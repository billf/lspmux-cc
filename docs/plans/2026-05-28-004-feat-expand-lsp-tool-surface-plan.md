---
title: "feat: Expand the MCP LSP tool surface (REV-010)"
status: active
date: 2026-05-28
deepened: 2026-06-01
type: feat
issue_id: REV-010
origin: todos/2026-05-28-expand-lsp-tool-surface.md
---

# feat: Expand the MCP LSP tool surface (REV-010)

## Summary

`mcp-server/src/tools.rs` exposes six tools (`rust_diagnostics`, `rust_hover`, `rust_goto_definition`, `rust_find_references`, `rust_workspace_symbol`, `rust_server_status`). Several rust-analyzer capabilities a human editor uses daily are unreachable by an agent, forcing manual edits, full-file reads, and "wait and retry" guessing. This plan adds new MCP tools — code actions, document symbols, rename, readiness signal, call hierarchy, go-to-implementation, expand-macro — each wrapping the corresponding LSP request and returning edits/locations as data without applying them. Expanding advertised `ClientCapabilities` (AGENT-7) lands first because rust-analyzer may withhold features otherwise.

---

## Problem Frame

The current six tools (verified locations: `rust_diagnostics` `tools.rs:309`, `rust_hover` `:387`, `rust_goto_definition` `:437`, `rust_find_references` `:496`, `rust_workspace_symbol` `:544`, `rust_server_status` `:603`) are registered via the rmcp `#[tool_router]` macro (`mcp-server/src/tools.rs:286-289`). `ClientCapabilities` is `ClientCapabilities::default()` with only an `experimental` serverStatus override (`mcp-server/src/lsp_client.rs:236-240`); default capabilities advertise little, so rust-analyzer may not offer code actions, rename, call hierarchy, etc.

Gaps from the 2026-03-18 review (AGENT-1..8):
- **AGENT-1 code actions:** no `textDocument/codeAction`; agent guesses instead of applying RA's quick fix.
- **AGENT-2 rename:** no `textDocument/rename`; cross-codebase renames need manual `find_references` + per-file edits.
- **AGENT-3 document symbols:** no `textDocument/documentSymbol`; can't outline a file without reading it.
- **AGENT-4 readiness:** largely shipped since this plan was drafted — `experimental/serverStatus` is ingested (`lsp_client.rs` `handle_server_status_notification`), tracked in `ReadinessState { health, quiescent, message, updated_at_ms }` (`telemetry.rs`), exposed via `LspClient::readiness()`, and surfaced through `rust_server_status` (`ServerStatusResponse.readiness` + the summary line). Residual only: no `$/progress` ingestion and no explicit `indexing` field (derivable as `!quiescent`). See U5.
- **AGENT-5 call hierarchy:** no `callHierarchy/incomingCalls` / `outgoingCalls`.
- **AGENT-6 go-to-implementation:** no `textDocument/implementation`.
- **AGENT-7 client capabilities:** prerequisite — must advertise the features the new tools need.
- **AGENT-8 expand-macro:** no `rust-analyzer/expandMacro`.

ARCH-1 (lib/bin split, `docs/brainstorms/2026-05-04-mcp-server-workspace-split-requirements.md`) is **not** done; `tools.rs` is binary-only today, so tool-dispatch integration tests are constrained until that split lands (see Risks).

---

## Requirements

- **R1.** `ClientCapabilities` advertises the features the new tools require (AGENT-7).
- **R2.** `rust_code_actions` returns available actions with their edits, unapplied (AGENT-1).
- **R3.** `rust_rename` returns the workspace edit without applying it (AGENT-2).
- **R4.** `rust_document_symbols` returns the file's symbol tree (AGENT-3).
- **R5.** `rust_server_status` reports indexing/readiness state (AGENT-4).
- **R6.** Call-hierarchy and go-to-implementation tools added (AGENT-5/6).
- **R7.** `rust_expand_macro` added (AGENT-8).
- **R8.** Each new tool has parameter-validation and response-shaping tests.

---

## Key Technical Decisions

**KTD1 — Capabilities first (AGENT-7 gates the rest).** Expand `ClientCapabilities` (`mcp-server/src/lsp_client.rs:236-240`) to advertise codeAction (with resolve + literal support), rename (prepareSupport), documentSymbol (hierarchical), callHierarchy, implementation, and the rust-analyzer experimental capabilities for `expandMacro`. Without this, RA may legitimately withhold the features and the new tools return empty.

**KTD2 — Tools return data, never apply edits.** `rust_code_actions` and `rust_rename` return the `WorkspaceEdit` / action set as structured JSON for the agent to inspect and apply via its own file edits. This matches the existing read-only posture of the six tools and keeps the MCP server from mutating the user's files.

**KTD3 — Incremental delivery by value order.** Land in the todo's suggested order: capabilities (U1) → code actions (U2) → document symbols (U3) → rename (U4) → readiness (U5) → call hierarchy / implementation (U6) → expand-macro (U7). Each tool is independently shippable after U1.

**KTD4 — Readiness via the already-wired serverStatus channel (already shipped).** `ClientCapabilities` overrides `experimental.serverStatusNotification` and the server ingests `experimental/serverStatus` into `ReadinessState`, surfacing it through `rust_server_status`. U5 is therefore mostly done; its residual is the optional `$/progress` channel and an explicit `indexing` field. The shipped readiness signal already unblocks REV-008's cache-poisoning guard.

**KTD5 — Mirror the existing tool pattern exactly.** New `#[tool]` methods follow the shape of `rust_goto_definition` / `rust_find_references`: validate path (`validate_file_path` `tools.rs:34`), `ensure_file_open`, send the typed LSP request via `LspClient::request` (`lsp_client.rs:270`), shape the response into a record struct (like `LocationRecord`/`RangeRecord` at `tools.rs:146-266`), map errors with `internal_error` (`tools.rs:51`).

---

## Implementation Units

### U1. Expand advertised ClientCapabilities (AGENT-7 — prerequisite)

**Goal:** RA offers code actions, rename, document symbols, call hierarchy, implementation, expand-macro.

**Requirements:** R1.

**Dependencies:** none.

**Files:**
- `mcp-server/src/lsp_client.rs` (`ClientCapabilities` construction ~236-240, initialize handshake ~227-262)
- `mcp-server/src/lsp_client.rs` tests / `mcp-server/tests/` capability assertion

**Approach:** Replace `ClientCapabilities::default()` with an explicit `TextDocumentClientCapabilities` advertising: `code_action` (with `code_action_literal_support` + `resolve_support`), `rename` (`prepare_support`), `document_symbol` (`hierarchical_document_symbol_support`), `call_hierarchy`, `implementation`, plus the rust-analyzer experimental `expandMacro` flag in `experimental`. Preserve the existing serverStatus experimental override.

**Test scenarios:**
- Happy path: serialized `initialize` params include the advertised capabilities (assert the JSON shape).
- Edge: existing `experimental.serverStatusNotification` override is preserved alongside the new experimental flags.
- Integration (gated): against a real RA, `initialize` result shows the corresponding `ServerCapabilities` are offered.

**Verification:** `initialize` advertises the new capabilities; serverStatus override intact.

### U2. `rust_code_actions` tool (AGENT-1)

**Goal:** Return available code actions + their edits for a file/range, unapplied.

**Requirements:** R2, R8.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/tools.rs` (new `#[tool]` method + a `CodeActionRecord` response struct near `:146-266`)
- `mcp-server/src/tools.rs` tests

**Approach:** Params: file path + range (or a diagnostic position). Validate path, `ensure_file_open`, send `textDocument/codeAction`, shape each action (title, kind, the `WorkspaceEdit` as data) into records. Do not apply.

**Patterns to follow:** `rust_find_references` (`tools.rs:496`) for path+position handling; `RangeRecord` (`tools.rs:266`) for range shaping.

**Test scenarios:**
- Covers R2: a response with two actions shapes into two records, each carrying its edit.
- Param validation: missing/invalid path → validation error (mirror existing tools).
- Edge: no actions available → empty array, not error.
- Edge: action with a command (not an edit) is represented distinctly from an edit-bearing action.

**Verification:** Tool returns actions with edits as data; validation tested.

### U3. `rust_document_symbols` tool (AGENT-3)

**Goal:** Return a file's hierarchical symbol tree.

**Requirements:** R4, R8.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/tools.rs` (new `#[tool]` + `DocumentSymbolRecord` nested struct)
- tests

**Approach:** Params: file path. `textDocument/documentSymbol`. Shape the hierarchical result (name, kind via `symbol_kind_name` `tools.rs:65`, range, children). Handle both `DocumentSymbol[]` (hierarchical) and `SymbolInformation[]` (flat) responses.

**Test scenarios:**
- Covers R4: a nested module → function tree shapes into nested records.
- Edge: flat `SymbolInformation[]` response still shapes correctly.
- Edge: empty file → empty symbol list.
- Param validation: invalid path → error.

**Verification:** Returns the symbol tree; both response shapes handled.

### U4. `rust_rename` tool (AGENT-2)

**Goal:** Return the workspace edit for a rename without applying it.

**Requirements:** R3, R8.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/tools.rs` (new `#[tool]` + `WorkspaceEditRecord`)
- tests

**Approach:** Params: file path, position, new name. Optionally call `textDocument/prepareRename` first to validate the position. Send `textDocument/rename`; return the `WorkspaceEdit` (per-file text edits) as data (KTD2). Never write files.

**Test scenarios:**
- Covers R3: rename returns edits spanning multiple files as records; nothing is written to disk (assert no file mutation).
- Edge: rename at an invalid position (e.g., on a keyword) → structured error, not panic.
- Param validation: empty new name → validation error.

**Verification:** Returns multi-file workspace edit as data; no disk writes.

### U5. Readiness/quiescence in `rust_server_status` (AGENT-4) — mostly shipped

**Status note:** The core of this unit already exists. `experimental/serverStatus`
is ingested by `handle_server_status_notification` (`mcp-server/src/lsp_client.rs`),
stored in `ReadinessState { health, quiescent, message, updated_at_ms }`
(`mcp-server/src/telemetry.rs`), exposed via `LspClient::readiness()`
(`mcp-server/src/lsp_client.rs`), and surfaced through `rust_server_status`
(`ServerStatusResponse.readiness` at `mcp-server/src/tools.rs`, plus the summary
line). A passing test (`server_status_notification_updates_readiness`) covers the
notification→state transition. ce-work should treat the shipped slice as done and
only address the residual below.

**Goal (residual):** Optionally widen the readiness signal — ingest `$/progress`
and/or add an explicit `indexing` field — and confirm the status response exposes
the readiness fields agents need. Skip entirely if the shipped `health` +
`quiescent` already satisfy R5 in practice.

**Requirements:** R5 (already substantially met), R8.

**Dependencies:** none for the residual (the serverStatus channel is already wired; it does not depend on U1's capability expansion).

**Files:**
- `mcp-server/src/lsp_client.rs` (only if adding `$/progress` handling alongside the existing `experimental/serverStatus` branch in `reader_loop`)
- `mcp-server/src/telemetry.rs` (only if adding an explicit `indexing` field to `ReadinessState`)
- tests

**Approach:** `indexing` is derivable as `!quiescent` today, so an explicit field is
a convenience, not a correctness gap. `$/progress` is partly redundant with
rust-analyzer's `serverStatus.quiescent`; add it only if a concrete agent need
appears. Any addition must stay additive to the serialized status contract.

**Test scenarios (residual only):**
- If `$/progress` ingestion is added: a `$/progress` "begin"→"end" sequence updates the readiness snapshot consistently with the serverStatus path.
- If an explicit `indexing` field is added: it equals `!quiescent` and defaults conservatively (treated as indexing) before any notification arrives.

**Verification:** No regression to the existing readiness surfacing; any new field is
additive and tested. If the residual is skipped, record that the shipped signal
satisfies R5.

### U6. Call hierarchy + go-to-implementation tools (AGENT-5/6)

**Goal:** `rust_call_hierarchy_incoming` / `_outgoing` and `rust_goto_implementation`.

**Requirements:** R6, R8.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/tools.rs` (new `#[tool]` methods + records)
- tests

**Approach:** Call hierarchy is two-step: `textDocument/prepareCallHierarchy` → `callHierarchy/incomingCalls` / `outgoingCalls`. Go-to-implementation: `textDocument/implementation` (shape like `rust_goto_definition` `tools.rs:437`).

**Test scenarios:**
- Covers R6: prepare→incoming returns caller locations as records; outgoing returns callees.
- Edge: position with no calls → empty result.
- Implementation: returns impl locations distinct from definition.
- Param validation for all three.

**Verification:** All three tools return locations as data.

### U7. `rust_expand_macro` tool (AGENT-8)

**Goal:** Expand a macro at a position.

**Requirements:** R7, R8.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/tools.rs` (new `#[tool]` + `MacroExpansionRecord`)
- tests

**Approach:** Send the rust-analyzer-specific `rust-analyzer/expandMacro` request (params: file + position). Return `{ name, expansion }`.

**Test scenarios:**
- Covers R7: expanding a `derive`/`macro_rules!` site returns the expansion text.
- Edge: position not on a macro → empty/None result, structured (not error).
- Param validation: invalid path/position.

**Verification:** Returns expansion text for a macro site.

---

## Scope Boundaries

In scope: AGENT-1..8 as read-only data-returning tools, plus the capability expansion that gates them.

### Deferred to Follow-Up Work
- Applying edits server-side (rename/code-action application) — intentionally out; tools return data only (KTD2).
- ARCH-1 lib/bin split — recommended to land first for richer tool-dispatch integration tests, but each tool ships with param-validation + response-shaping tests regardless (see Risks).

Out of scope: non-Rust language support; UI for presenting actions.

---

## Risks & Dependencies

- **Testability gated by ARCH-1:** `tools.rs` is binary-only today, so full tool-dispatch integration tests are limited. Mitigation: each unit ships param-validation + response-shaping unit tests against shaped LSP payloads; deeper dispatch tests follow ARCH-1. Flagged, not blocking.
- **Capability advertisement is load-bearing (U1):** if a capability is mis-advertised, the dependent tool silently returns empty. Each tool's integration test (gated) should confirm RA actually offers the feature.
- **rust-analyzer experimental requests** (`expandMacro`, `serverStatus`) are non-standard LSP; pin behavior against the RA version this repo ships (fenix nightly via the flake).
- Synergy: U5 readiness unblocks REV-008's cache-poisoning mitigation.

---

## Verification

1. `cargo test --manifest-path mcp-server/Cargo.toml` covers param validation + response shaping for every new tool (R8).
2. `cargo build` succeeds; `initialize` advertises the expanded capabilities (U1 test).
3. Gated integration tests (real RA) confirm each capability is offered and each tool returns non-empty data on a fixture.
4. `rust_server_status` reports indexing/quiescent/health (U5).
5. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` clean.

---

## Sources & Research

- Origin todo: `todos/2026-05-28-expand-lsp-tool-surface.md` (REV-010; consolidates AGENT-1..8).
- Verified: six tools at `mcp-server/src/tools.rs:309/387/437/496/544/603`; rmcp `#[tool_router]` `tools.rs:286-289`; `ClientCapabilities::default()` + serverStatus override `mcp-server/src/lsp_client.rs:236-240`; `LspClient::request` `:270`; record structs `tools.rs:146-266`; `RuntimeStatus` `bootstrap.rs:150-188`.
- ARCH-1 dependency context: `docs/brainstorms/2026-05-04-mcp-server-workspace-split-requirements.md` (not yet done).
