# P3: Cleanup Findings

Full codebase review, 2026-03-18. Branch: `main` (HEAD: 0d7e1e2).
Sources: code-simplicity-reviewer, pattern-recognition-specialist, clippy.

---

## CLIP-1: Three `missing_const_for_fn` warnings

**Files:** `mcp-server/src/tools.rs`, lines 46, 60, 245
**Source:** clippy (nursery lint)

Functions `internal_error`, `symbol_kind_name`, and `range_record` could be `const fn`.

**Fix:** Add `const` keyword to each. Or suppress if the const-ness isn't valuable.

---

## SIMP-3: Remove trivial tests that test the standard library

**File:** `mcp-server/src/lsp_client.rs`, lines 650-681
**Source:** code-simplicity-reviewer (9)

Five tests (32 lines) that test `AtomicBool`, `tokio::sync::Mutex<Option<String>>`, not `LspClient`:
- `is_alive_reflects_atomic_state`
- `workspace_root_returns_none_when_unset`
- `workspace_root_returns_value_when_set`
- `server_version_returns_none_when_unset`
- `server_version_returns_value_when_set`

**Fix:** Remove. Zero confidence loss about actual behavior.

---

## SIMP-4: Trim unnecessary derives on response-only structs

**File:** `mcp-server/src/tools.rs`, lines 141-225
**Source:** code-simplicity-reviewer (1)

Response structs derive `Deserialize`, `Clone`, `PartialEq`, `Eq` but these are never deserialized, never cloned, and no tests compare them. Only `JsonSchema`, `Serialize`, `Debug` are needed.

**Fix:** Remove unused derives from the 7 response structs (DiagnosticsResponse, HoverResponse, LocationsResponse, WorkspaceSymbolsResponse, ServerStatusResponse, LocationRecord, DiagnosticRecord, etc.). Note: check if `Clone` is needed by `RuntimeStatus` (it's stored in `RustAnalyzerTools`).

---

## PAT-1: Error message format inconsistency in shell scripts

**Source:** pattern-recognition-specialist

Shell scripts use three different error prefixes:
- `error:` (lowercase) in wrapper scripts
- `ERROR:` (uppercase) in `setup` (`die()` function)
- `WARNING:` (uppercase) in hooks

**Fix:** Standardize on `error:` (lowercase, matching rust/cargo convention) across all scripts.

---

## PAT-2: Binary resolution logic duplicated across 4 locations

**Source:** pattern-recognition-specialist

The lspmux binary resolution cascade (env var -> PATH -> cargo home) is implemented in:
1. `mcp-server/src/bootstrap.rs` (Rust)
2. `plugins/lspmux-rust-cc/bin/lspmux` (shell wrapper)
3. `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh`
4. `setup` (implicit via `command -v`)

The rust-analyzer resolution has a similar 3-way split.

**Fix:** Acceptable duplication given the different execution contexts (Rust vs shell). Document the canonical resolution order in one place and reference it.

---

## PAT-5: `session-start.sh` systemMessage string duplicated 3 times

**File:** `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh`, lines 28, 40, 56
**Source:** pattern-recognition-specialist (3.5)

The tool list and coordinate convention string appears three times. If a tool is added or renamed, three lines need updating.

**Fix:** Extract to a variable at the top of the script.

---

## PAT-6: Tool preamble (validate + ensure_file_open) duplicated 4 times

**File:** `mcp-server/src/tools.rs`
**Source:** pattern-recognition-specialist (3.1)

```rust
validate_file_path(&p.file_path)?;
self.lsp.ensure_file_open(&p.file_path).await.map_err(|e| internal_error(...))?;
```

Appears in `diagnostics`, `hover`, `goto_definition`, and `find_references`. Extract to a helper method.

**Fix:** Add `ensure_file_synced(&self, path: &str) -> Result<(), McpError>` that combines both steps.

---

## PERF-3: `validate_file_path()` calls blocking `Path::exists()` in async context

**File:** `mcp-server/src/tools.rs`, lines 29-44
**Source:** performance-oracle (7)

`p.exists()` is a blocking filesystem call on an async thread. Sub-millisecond for local SSD, won't cause executor starvation in practice.

**Fix:** Low priority. Use `tokio::fs::metadata()` if strictness desired.

---

## SEC-6: `sed` template substitution fragile against special characters

**File:** `setup`, lines 74, 85-89, 97-101
**Source:** security-sentinel (LOW-1)

`sed` substitutions use `|` as delimiter, but replacement values from env vars or `command -v` could contain `|`, `&`, or `\`. Unlikely in practice but fragile pattern.

**Fix:** Use `envsubst` or escape replacement strings.
