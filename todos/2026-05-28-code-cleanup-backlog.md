---
status: pending
priority: p3
issue_id: "REV-012"
tags: [cleanup, simplification, performance, patterns, clippy]
dependencies: []
---

# Code cleanup backlog

A single backlog for the low-risk cleanup findings from the 2026-03-18 review
(archived under `todos/archive/`). Each item points back to its origin finding.
Pick off opportunistically; none block feature work.

## Items

| Item | Origin | File(s) | Action |
|------|--------|---------|--------|
| OnceCell for write-once fields | PERF-1 (p2) | `mcp-server/src/lsp_client.rs:50-52` | Replace `Mutex<Option<String>>` (`workspace_root`, `server_version`) with `OnceLock`/`OnceCell` |
| Shrink `MAX_LSP_MESSAGE_SIZE` | SIMP-1 (p2) | `mcp-server/src/lsp_client.rs:36` | 100MB → 10MB |
| Trim `detect_language_id` | SIMP-2 (p2) | `mcp-server/src/lsp_client.rs:96-124` | Keep `rs`, `toml`, fallback `plaintext` |
| Coalesce `send_message` writes | PERF-2 (p2) | `mcp-server/src/lsp_client.rs:311-317` | One buffer write instead of header+body+flush |
| `const fn` candidates | CLIP-1 | `mcp-server/src/tools.rs:46,60,245` | `internal_error`, `symbol_kind_name`, `range_record` |
| Drop stdlib-only tests | SIMP-3 | `mcp-server/src/lsp_client.rs:650-681` | Remove 5 tests that exercise `AtomicBool`/`Mutex`, not `LspClient` |
| Trim response-struct derives | SIMP-4 | `mcp-server/src/tools.rs:141-225` | Drop unused `Deserialize`/`Clone`/`PartialEq`/`Eq` (check `RuntimeStatus` `Clone`) |
| Standardize shell error prefix | PAT-1 | shell scripts | Settle on lowercase `error:` (rust/cargo convention) |
| Document binary-resolution order | PAT-2 | `bootstrap.rs`, `bin/`, `setup` | Duplication is acceptable; document the canonical cascade once |
| Extract tool preamble helper | PAT-6 | `mcp-server/src/tools.rs` | `ensure_file_synced()` combining validate + `ensure_file_open` |
| `validate_file_path` blocking call | PERF-3 | `mcp-server/src/tools.rs:29-44` | Low priority; `tokio::fs::metadata` if strictness wanted |
| Fragile `sed` substitution | SEC-6 | `setup:74,85-89,97-101` | Use `envsubst` or escape replacement strings |

## Acceptance Criteria

- [ ] Each item is either applied or consciously closed as won't-fix with a note
- [ ] `cargo clippy -W clippy::nursery -W clippy::pedantic` stays clean
- [ ] `just shellcheck` passes after shell-script changes

## Work Log

### 2026-05-28 - Promoted from review snapshots

**By:** Claude Code (todos/brainstorms consolidation)

**Actions:**
- Collected the P2/P3 cleanup findings into one opportunistic backlog so the
  review files could be archived without losing the work items.
