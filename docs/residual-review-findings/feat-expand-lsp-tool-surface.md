# Residual Review Findings — feat/expand-lsp-tool-surface (REV-010)

Source: ce-code-review run `20260601-0ce547df` (autofix), 7 personas + Codex, on
`git diff 211c4cf..HEAD`. Verdict: **Ready with fixes** — 10 safe_auto fixes were
applied in `fix(review): apply autofix feedback`. KTD2 (tools never mutate files)
independently confirmed by the adversarial reviewer and Codex.

The items below are non-blocking follow-ups, recorded here because no PR is open
yet. None are filed as tracker tickets (that needs explicit approval).

## Maintainability
- **[P1] `mcp-server/src/tools.rs` is ~1900 lines.** Extract record structs into
  `tools/records.rs` and shaping helpers into `tools/shaping.rs`. Does **not**
  require the ARCH-1 lib/bin split.
Resolved by the `refactor(tools): dedupe tool preludes and goto shaping` pass:
- ~~Repeated tool prelude~~ — extracted `open_file_uri`.
- ~~Duplicated goto-response shaping~~ — extracted `locations_from_goto_response`.
- ~~`CallHierarchyResponse.direction` stringly-typed~~ — now a `CallDirection` enum.
- ~~`rust_rename` kept its own prelude~~ — now reuses `open_file_uri` after the
  empty-name guard moves to the top.

## Correctness / contract
- **[P2] Resource operations silently dropped.** `workspace_edit_record` skips
  `DocumentChanges::Operations` resource ops (create/rename/delete file). A rename
  refactor that creates a file returns only text edits with no signal. Surface a
  count or list so an agent applying the edit doesn't miss the file op.
- ~~**[P3] `rust_code_actions` sends an empty diagnostics context.**~~ Resolved:
  the tool description now states that diagnostic-triggered quick fixes may be
  absent because no diagnostics context is sent. (Fetching live diagnostics to
  populate `CodeActionContext` remains a possible future upgrade.)
- **[P2] `prepare_call_hierarchy` keeps only the first prepare item.** When a
  position is ambiguous (e.g. macro-generated methods), extra anchors are dropped
  with no signal to the caller.

## Robustness
- ~~**[P3] Unbounded `document_symbol_record` recursion.**~~ Resolved: shaping is
  capped at `MAX_SYMBOL_DEPTH` (64); children below the cap are dropped so a
  degenerate tree can't overflow the stack. Covered by
  `document_symbol_record_caps_deep_nesting`.

## Testing (deferred to ARCH-1)
- **[deferred] Tool-method-level tests.** The async `#[tool]` methods (including
  the `rust_rename` empty-name guard) can't be unit-tested without a live
  rust-analyzer until ARCH-1 introduces a lib/bin split or a mock `LspClient`.
  Current coverage is param-validation + response-shaping against constructed
  payloads, per the plan's Risk note.
