# P2: Important Findings

Full codebase review, 2026-03-18. Branch: `main` (HEAD: 0d7e1e2).
Sources: security-sentinel, performance-oracle, architecture-strategist, code-simplicity-reviewer, agent-native-reviewer, pattern-recognition-specialist.

---

## SEC-3: Socket existence check without type verification

**File:** `mcp-server/src/bootstrap.rs`, lines 203-205
**Source:** security-sentinel (MEDIUM-1)

`socket_ready()` checks `Path::exists()` but not that the path is actually a Unix socket. A stale regular file or symlink satisfies the check, causing the server to skip startup while all LSP connections fail. The `setup` script's `cmd_doctor()` correctly uses `[ -S ... ]`.

**Fix:** Use `std::fs::metadata` and check `file_type().is_socket()`:

```rust
fn socket_ready(&self) -> bool {
    std::fs::metadata(&self.socket_path)
        .map(|m| m.file_type().is_socket() || m.file_type().is_fifo())
        .unwrap_or(false)
}
```

(Note: on macOS, `is_socket()` may not work through symlinks. Test accordingly.)

---

## ARCH-2: Bootstrap redundancy between Rust and shell

**Files:** `mcp-server/src/bootstrap.rs` and `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh`
**Source:** architecture-strategist (4a), code-simplicity-reviewer (5)

Both implement the same 3-tier cascade: check socket -> try service manager -> start directly. The MCP server's `ensure_service_running()` handles this on startup. The hook runs *before* the MCP server starts, creating a race where both try to start the service.

The hook also embeds a hardcoded tool list and coordinate format in its `systemMessage`, becoming a maintenance liability when tools change.

`docs/hosts/claude-code.md:28` says: "Hooks are optional optimization only."

**Fix:** Strip session-start.sh to a simple status check + systemMessage. Don't reimplement bootstrap. ~35 LOC savings.

---

## ARCH-3: `lsp_client.rs` doing too many jobs (714 lines, 6 concerns)

**File:** `mcp-server/src/lsp_client.rs`
**Source:** architecture-strategist (2b)

Handles: JSON-RPC framing, request/response multiplexing, LSP lifecycle, file synchronization, URI encoding/decoding, language detection.

**Fix:** Extract at minimum:
- `uri.rs`: `file_uri`, `uri_to_path`, `PATH_ENCODE_SET` (pure functions, already tested independently)
- `language.rs`: `detect_language_id` (pure function, already tested independently)

Mechanical extraction, zero behavioral change.

---

## PERF-1: Write-once fields behind Mutex

**File:** `mcp-server/src/lsp_client.rs`, lines 50-52
**Source:** pattern-recognition-specialist (2.3), performance-oracle

```rust
workspace_root: tokio::sync::Mutex<Option<String>>,
server_version: tokio::sync::Mutex<Option<String>>,
```

Written once during `new()` (lines 228-229), only read afterward. Mutex is heavier than needed.

**Fix:** Use `tokio::sync::OnceCell<String>` or `std::sync::OnceLock<String>`.

---

## SIMP-1: Reduce `MAX_LSP_MESSAGE_SIZE` from 100MB to 10MB

**File:** `mcp-server/src/lsp_client.rs`, line 36
**Source:** code-simplicity-reviewer (7)

Rust-analyzer's largest realistic responses are low-MB range. 100MB means a malformed `Content-Length` header causes a 100MB allocation before failing.

**Fix:** Change to `10 * 1024 * 1024`.

---

## SIMP-2: Trim `detect_language_id()` to relevant entries

**File:** `mcp-server/src/lsp_client.rs`, lines 96-124
**Source:** code-simplicity-reviewer (2)

25+ language mappings for a server that talks exclusively to rust-analyzer. Realistic file types: `.rs`, `.toml`, maybe `.json`. Python, Go, Ruby, CSS, HTML, SQL, JSX, TSX, C++ entries will never produce useful RA results.

**Fix:** Trim to `rs`, `toml`, and `_ => "plaintext"` fallback. ~21 LOC savings in impl, ~16 LOC in tests.

---

## AGENT-4: No "wait for ready" mechanism after edits

**File:** SKILL.md line 49, main.rs instructions line 55
**Source:** agent-native-reviewer (Warning-1)

Instructions say "wait a few seconds and retry." The agent has no way to know when RA finishes indexing.

**Fix:** Expose RA's `experimental/serverStatus` or `$/progress` notifications. Enhance `rust_server_status` to return `{ "indexing": true/false }`.

---

## AGENT-5: No call hierarchy tool

**File:** gap in `mcp-server/src/tools.rs`
**Source:** agent-native-reviewer (Warning-2)

Agent can use `rust_find_references` for incoming calls (rough approximation) but can't trace outgoing calls at all.

**Fix:** Add `rust_incoming_calls` and `rust_outgoing_calls` wrapping `callHierarchy/incomingCalls` and `callHierarchy/outgoingCalls`.

---

## AGENT-6: No go-to-implementation tool

**File:** gap in `mcp-server/src/tools.rs`
**Source:** agent-native-reviewer (Warning-3)

Given a trait, a human can jump to all implementations. `rust_find_references` returns uses, which is different from implementations.

**Fix:** Add `rust_goto_implementation(file_path, line, character)` wrapping `textDocument/implementation`.

---

## AGENT-7: `ClientCapabilities::default()` limits what RA offers

**File:** `mcp-server/src/lsp_client.rs`, line 218
**Source:** agent-native-reviewer (Warning-4)

The LSP client advertises `ClientCapabilities::default()`, which may cause RA to withhold capabilities it would otherwise offer (code actions, call hierarchy, etc.). When adding new tools, the client capabilities must be expanded to advertise support for the corresponding LSP features.

**Fix:** Populate `ClientCapabilities` with the specific capabilities needed for the tools being exposed.

---

## SEC-4: Detached server process with no PID tracking

**File:** `mcp-server/src/bootstrap.rs`, lines 253-266
**Source:** security-sentinel (MEDIUM-3)

`start_direct_server()` spawns `lspmux server` with null stdout/stderr, then immediately drops the child handle. No `kill_on_drop`, no PID stored. Multiple invocations could start multiple servers racing for the same socket. Orphaned processes accumulate over time.

**Fix:** Store PID in a pidfile alongside the socket. Check for existing pidfile before spawning. Log to a file instead of /dev/null.

---

## SEC-5: Socket directory created without restrictive permissions

**File:** `setup`, line 48
**Source:** security-sentinel (MEDIUM-4)

`mkdir -p` creates the socket directory with default permissions (0755). When `XDG_RUNTIME_DIR` is unset and `TMPDIR=/tmp`, the socket at `/tmp/lspmux/lspmux.sock` is world-accessible. Other local users could connect and issue LSP commands.

**Fix:** Add `chmod 700 "${LSPMUX_SOCKET_DIR}"` after `mkdir -p`.

---

## PAT-3: Missing `--config` flag in session-start.sh direct server start

**File:** `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh`, line 52
**Source:** pattern-recognition-specialist (2.6)

```bash
"${LSPMUX_BIN}" server &
```

No `--config` flag, unlike the Rust equivalent at `bootstrap.rs:254-258` which passes `--config` explicitly. If config is in a non-default location, the shell path silently uses wrong settings.

**Fix:** Pass `--config` explicitly, matching the Rust behavior.

---

## PAT-4: `update-rust-analyzer` only supports macOS

**File:** `bin/update-rust-analyzer`, lines 12-19
**Source:** pattern-recognition-specialist (2.7)

Architecture detection only handles `aarch64-apple-darwin` and `x86_64-apple-darwin`. Validates downloaded binary as Mach-O. CLAUDE.md mentions Linux support, but a Linux user running `./setup core` gets a confusing error.

**Fix:** Add Linux platform detection (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`) and use `file` check for ELF instead of Mach-O.

---

## PERF-2: `send_message()` performs 3 separate writes per message

**File:** `mcp-server/src/lsp_client.rs`, lines 311-317
**Source:** performance-oracle (5)

Three separate async writes (header, body, flush) while holding the stdin lock. Coalescing into a single pre-allocated buffer reduces syscalls and lock hold time.

**Fix:** Concatenate header + body into a single `Vec` before writing.

---

## AGENT-8: No expand-macro tool

**File:** gap in `mcp-server/src/tools.rs`
**Source:** agent-native-reviewer (Warning-5)

Macro-heavy Rust code is hard to reason about. RA's `rust-analyzer/expandMacro` custom request shows what `#[derive(...)]` or `macro_rules!` expands to. Particularly high value for an AI agent.

**Fix:** Add `rust_expand_macro(file_path, line, character)` wrapping RA's custom request.

---

## Work Log

### 2026-03-24 - Current state

- `mcp-server/src/bootstrap.rs` already checks socket readiness by socket type rather than path existence, and `nix_like_uid()` already uses the OS uid directly.
- Regression tests now pin those bootstrap invariants so the stale security guidance in this review is less likely to regress.
- The remaining items in this file are the broader bootstrap-race and LSP-transport follow-ups that are intentionally outside Task 1.
