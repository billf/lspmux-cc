---
title: "refactor: Code cleanup backlog (REV-012)"
status: active
date: 2026-05-28
type: refactor
issue_id: REV-012
origin: todos/2026-05-28-code-cleanup-backlog.md
---

# refactor: Code cleanup backlog (REV-012)

## Summary

A backlog of low-risk cleanup findings from the 2026-03-18 review, none blocking feature work. Verification against current code shows several items are **already done or moot** (two of the three `const fn` candidates already are `const fn`; the config-write `sed` already uses a safe delimiter and non-user input). This plan executes the still-valid items grouped by file area, and explicitly closes the already-satisfied ones with a note — satisfying the backlog's own acceptance criterion that each item is either applied or consciously closed. The bar throughout: `cargo clippy -W clippy::nursery -W clippy::pedantic` stays clean and `just shellcheck` passes.

---

## Problem Frame

The origin backlog lists 12 items (PERF-1/2/3, SIMP-1/2/3/4, CLIP-1, PAT-1/2/6, SEC-6). Current-code verification:

| Item | Origin | Status now | Location (verified) |
|------|--------|-----------|---------------------|
| OnceCell for write-once fields | PERF-1 | valid (async nuance) | `lsp_client.rs:54,56` use `tokio::sync::Mutex<Option<String>>` |
| Shrink `MAX_LSP_MESSAGE_SIZE` 100MB→10MB | SIMP-1 | valid | `lsp_client.rs:40` = `100 * 1024 * 1024` |
| Trim `detect_language_id` | SIMP-2 | valid | `lsp_client.rs:102-130` (28 lines, plaintext fallback) |
| Coalesce `send_message` writes | PERF-2 | valid | `lsp_client.rs:329-343` |
| `const fn` candidates | CLIP-1 | **mostly done** | `symbol_kind_name:65` and `range_record:266` are **already `const fn`**; `internal_error:51` is the only remaining candidate (and may not be const-able) |
| Drop stdlib-only tests | SIMP-3 | valid | `lsp_client.rs:720-780` |
| Trim response-struct derives | SIMP-4 | needs verification | `tools.rs:146-234` uniform `Clone/Debug/Deserialize/Serialize/JsonSchema/PartialEq/Eq` — drop only genuinely-unused ones |
| Standardize shell error prefix | PAT-1 | valid | shell scripts |
| Document binary-resolution order | PAT-2 | valid | `bin/lspmux` (3-tier), `bin/lspmux-cc-mcp` (5-tier), `bin/rust-analyzer` (2-tier) |
| Extract tool preamble helper | PAT-6 | valid | `tools.rs` (no `ensure_file_synced` exists) |
| `validate_file_path` blocking call | PERF-3 | valid (low priority) | `tools.rs:34` |
| Fragile `sed` substitution | SEC-6 | **likely moot** | config-write `sed` at `setup:91-92` uses `\|` delimiter + non-user input; re-verify the originally-cited lines |

---

## Requirements

- **R1.** Each backlog item is either applied or consciously closed as won't-fix with a note.
- **R2.** `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` stays clean.
- **R3.** `just shellcheck` passes after shell-script changes.

---

## Key Technical Decisions

**KTD1 — `tokio::sync::OnceCell`, not `std::OnceLock`, for PERF-1.** `workspace_root` / `server_version` are set during the async initialize handshake, not at construction. `std::sync::OnceLock` is fine only if the set happens synchronously; since these are written from async code after `initialize`, `tokio::sync::OnceCell<String>` is the correct write-once primitive (or `OnceLock` if the set point is actually synchronous — confirm at implementation). This removes the `Mutex<Option<_>>` lock on every read.

**KTD2 — Close already-satisfied items explicitly.** CLIP-1 is mostly done (`symbol_kind_name`, `range_record` already `const fn`); only `internal_error` remains and may not be const-able (depends on whether it allocates). SEC-6's main `sed` is already safe. Per R1, document these as closed rather than churning code.

**KTD3 — Derive trimming is evidence-based (SIMP-4).** Only remove a derive after confirming it's genuinely unused (no `==`, no `.clone()`, no deserialization of that type). Response structs are serialized **out**, so `Deserialize` is the prime suspect; `PartialEq`/`Eq`/`Clone` likely unused unless tests compare them. Do not blanket-strip.

**KTD4 — Group by file to minimize churn and keep commits coherent.** lsp_client.rs cleanups, tools.rs cleanups, shell cleanups, and docs as separate units.

---

## Implementation Units

### U1. `lsp_client.rs` structural cleanups (PERF-1, SIMP-1, SIMP-2, PERF-2)

**Goal:** Remove write-once locks, shrink the message cap, trim language detection, coalesce socket writes.

**Requirements:** R1, R2.

**Dependencies:** none.

**Files:**
- `mcp-server/src/lsp_client.rs` (fields ~54,56; `MAX_LSP_MESSAGE_SIZE` ~40; `detect_language_id` ~102-130; `send_message` ~329-343)
- `mcp-server/src/lsp_client.rs` tests

**Approach:**
- PERF-1: replace `tokio::sync::Mutex<Option<String>>` for `workspace_root`/`server_version` with `tokio::sync::OnceCell<String>` (KTD1); update read/write sites.
- SIMP-1: `MAX_LSP_MESSAGE_SIZE` 100MB → 10MB.
- SIMP-2: trim `detect_language_id` to `rs`, `toml`, fallback `plaintext`.
- PERF-2: coalesce `send_message`'s header+body+flush into a single buffered write.

**Patterns to follow:** existing async field access in `lsp_client.rs`; the framed write already in `send_message`.

**Test scenarios:**
- PERF-1: setting then reading `workspace_root` returns the value; second set is a no-op / panics-free per OnceCell semantics (assert chosen behavior).
- SIMP-1: a message just under 10MB is accepted; one over is rejected (assert new cap boundary).
- SIMP-2: `.rs`→rust, `.toml`→toml, unknown→plaintext.
- PERF-2: a sent message is received byte-identical (header + blank line + body) after coalescing.
- Edge: empty body still frames correctly.

**Verification:** Behavior unchanged externally; clippy clean; the read-path lock is gone.

### U2. `tools.rs` cleanups (CLIP-1 remainder, SIMP-4, PAT-6, PERF-3)

**Goal:** Finish const-fn where possible, trim genuinely-unused derives, extract a sync preamble helper, optionally async-ify path validation.

**Requirements:** R1, R2.

**Dependencies:** none.

**Files:**
- `mcp-server/src/tools.rs` (`internal_error:51`; response structs `146-234`; new `ensure_file_synced` helper; `validate_file_path:34`)
- `mcp-server/src/tools.rs` tests

**Approach:**
- CLIP-1: make `internal_error` `const fn` **if** it doesn't allocate; otherwise close as won't-fix with a note (KTD2). Confirm `symbol_kind_name`/`range_record` remain `const fn` (already are).
- SIMP-4: audit each response struct's derives; drop only confirmed-unused ones (KTD3) — likely `Deserialize` on output-only structs. Keep `RuntimeStatus` derives it actually needs.
- PAT-6: extract `ensure_file_synced()` combining `validate_file_path` + `ensure_file_open`, and call it from the tool methods that currently inline both.
- PERF-3 (low priority): switch `validate_file_path` to `tokio::fs::metadata` if a non-blocking check is wanted; otherwise close as won't-fix (it's a fast stat).

**Test scenarios:**
- SIMP-4: code compiles after derive removal (the real test — an unused derive removal is behavior-neutral); if a dropped derive was actually used, the build fails — keep it.
- PAT-6: tools using `ensure_file_synced` still validate bad paths (invalid path → same error as before) and still open the file.
- PERF-3 (if applied): valid path passes, missing path errors, via the async stat.
- Test expectation for pure derive trims with no behavioral change: covered by compilation + existing tool tests.

**Verification:** clippy clean; tool behavior unchanged; preamble helper used in place of duplicated validate+open.

### U3. Remove stdlib-only tests (SIMP-3)

**Goal:** Delete tests that exercise `AtomicBool`/`Mutex` rather than `LspClient`.

**Requirements:** R1, R2.

**Dependencies:** none.

**Files:**
- `mcp-server/src/lsp_client.rs` (~720-780, the 5 stdlib-only tests)

**Approach:** Remove the 5 tests that assert standard-library behavior, not `LspClient` logic. Confirm no unique coverage is lost (they test language/library primitives, not this crate).

**Test expectation:** none — this unit *removes* tests; verification is that the remaining suite still passes and covers `LspClient` behavior.

**Verification:** `cargo test` passes; removed tests contributed no `LspClient`-specific coverage.

### U4. Shell-script cleanups (PAT-1, SEC-6)

**Goal:** Standardize error prefix; resolve or close the `sed` fragility.

**Requirements:** R1, R3.

**Dependencies:** none.

**Files:**
- shell scripts (repo root `setup`, `bin/*`)

**Approach:**
- PAT-1: settle on lowercase `error:` prefix (rust/cargo convention) across shell scripts; update message strings.
- SEC-6: re-verify the originally-cited `sed` lines. The config-write `sed` (`setup:91-92`) already uses a `\|` delimiter on non-user input — close as already-safe (KTD2). If any *other* cited `sed` writes user-influenced replacement text, switch to `envsubst` or escape the replacement.

**Test scenarios:**
- Covers R3: `just shellcheck` passes after changes.
- PAT-1: a triggered error path prints the lowercase `error:` prefix.

**Verification:** shellcheck clean; error prefixes consistent; SEC-6 either fixed or documented as already-safe.

### U5. Document the binary-resolution cascade (PAT-2)

**Goal:** Document the canonical binary-resolution order once; accept the duplication.

**Requirements:** R1.

**Dependencies:** none.

**Files:**
- `docs/hosts/*.md` or a short section in `README.md` / a `docs/` note
- (reference, not change: `mcp-server/src/bootstrap.rs`, `bin/lspmux`, `bin/lspmux-cc-mcp`, `bin/rust-analyzer`, `setup`)

**Approach:** Document each binary's resolution cascade (verified): `lspmux` = `$LSPMUX_PATH` → PATH → `$CARGO_HOME/bin`; `lspmux-cc-mcp` = `$CLAUDE_PLUGIN_ROOT/bin` → `$LSPMUX_MCP_PATH` → PATH → `$CARGO_HOME/bin` → Nix `result/bin`; `rust-analyzer` = `$RUST_ANALYZER_PATH` → PATH. Note the override risk (env vars can shadow installed binaries). Duplication across `bootstrap.rs`/`bin/`/`setup` is acceptable; the doc is the single source of truth.

**Test expectation:** none — documentation only.

**Verification:** Doc captures all three cascades and the override-precedence risk.

---

## Scope Boundaries

In scope: the 12 backlog items, each applied or explicitly closed.

### Deferred to Follow-Up Work
- Any item that grows beyond low-risk during implementation (e.g., if derive-trimming reveals a real API coupling) is split out rather than forced into this refactor.

Out of scope: behavioral changes, new features, the ARCH-1 lib/bin split (separate effort).

---

## Risks & Dependencies

- **Derive removal (SIMP-4) can break the build** if a derive is actually used — that's the safety net (compilation fails), so trim conservatively (KTD3).
- **OnceCell semantics (PERF-1):** a double-set that previously overwrote silently will now behave differently; confirm the field is genuinely write-once before switching.
- **No cross-todo dependencies.** Items are independent and can land in any order / separate commits.

---

## Verification

1. `cargo test --manifest-path mcp-server/Cargo.toml` passes after each unit.
2. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` stays clean (R2).
3. `just shellcheck` passes (R3).
4. Each of the 12 items is traceable to either a code change or a recorded won't-fix/already-done note (R1) — the table in Problem Frame plus per-unit notes is that ledger.

---

## Sources & Research

- Origin todo: `todos/2026-05-28-code-cleanup-backlog.md` (REV-012; PERF-1/2/3, SIMP-1/2/3/4, CLIP-1, PAT-1/2/6, SEC-6 from the archived 2026-03-18 review).
- Verified: `lsp_client.rs` fields `:54,56`, `MAX_LSP_MESSAGE_SIZE :40`, `detect_language_id :102-130`, `send_message :329-343`, stdlib tests `:720-780`; `tools.rs` `internal_error :51`, `symbol_kind_name :65` (already `const fn`), `range_record :266` (already `const fn`), response structs `:146-234`, `validate_file_path :34`; `setup` config-write `sed :91-92` (already safe delimiter); `bin/` resolution cascades (3/5/2-tier).
