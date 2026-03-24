# P1: Critical Findings

Full codebase review, 2026-03-18. Branch: `main` (HEAD: 0d7e1e2).
Sources: security-sentinel, architecture-strategist, agent-native-reviewer.

---

## SEC-1: Binary download without upstream checksum verification

**File:** `bin/update-rust-analyzer`, lines 49-74
**Source:** security-sentinel (HIGH-1)

The script downloads a gzipped binary from GitHub, validates it's Mach-O, records SHA256 locally, but never verifies that checksum against an upstream known-good value. The `SHA256SUM` file is written *after* downloading, proving nothing about integrity.

An attacker who can MITM the GitHub CDN could serve a valid Mach-O binary with malicious code. The launchd/systemd timer runs this daily at 04:00, so a persistent compromise refreshes automatically.

**Fix:** Download the `SHA256SUMS` file from the GitHub release (rust-analyzer publishes one). Verify computed checksum against the upstream value before moving the binary into place.

```bash
CHECKSUMS_URL="$(echo "${RELEASE_JSON}" | jq -r '.assets[] | select(.name == "SHA256SUMS") | .browser_download_url')"
curl --proto =https --fail -sL "${CHECKSUMS_URL}" -o "${TMPFILE}.sums"
EXPECTED="$(grep "${ASSET_NAME%.gz}" "${TMPFILE}.sums" | cut -d' ' -f1)"
if [ "${CHECKSUM}" != "${EXPECTED}" ]; then
    echo "error: checksum mismatch" >&2
    exit 1
fi
```

---

## SEC-2: UID fallback to root (UID=0) in `nix_like_uid()`

**File:** `mcp-server/src/bootstrap.rs`, lines 275-280
**Source:** security-sentinel (HIGH-2)

```rust
fn nix_like_uid() -> u32 {
    std::env::var("UID")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(0)
}
```

`UID` is a bash special variable, not a standard env var. The Rust runtime won't inherit it. This *always* falls back to 0 unless explicitly set, causing `launchctl bootstrap gui/0` to target the root GUI session. On macOS it fails silently for non-root users, masking the bug via the `start_direct_server()` fallback.

**Fix:** Use `libc::getuid()` instead of reading the env var:

```rust
fn nix_like_uid() -> u32 {
    // SAFETY: getuid() is a safe POSIX syscall that always succeeds.
    unsafe { libc::getuid() }
}
```

Add `libc` to dependencies (or use `nix::unistd::getuid()`).

---

## ARCH-1: `tools.rs` trapped in binary crate

**File:** `mcp-server/src/main.rs`, line 8 (`mod tools;`)
**Source:** architecture-strategist (2a)

702 lines of tool definitions (the most important contract surface) are invisible to `lib.rs`. Integration tests can't exercise `RustAnalyzerTools`, tool dispatch, parameter validation, response formatting, or MCP error boundaries.

**Fix:** Move `tools.rs` into the library crate:

```rust
// lib.rs
pub mod bootstrap;
pub mod lsp_client;
pub mod tools;
```

Binary's `main.rs` imports `lspmux_cc_mcp::tools::RustAnalyzerTools` instead of `crate::tools`.

---

## AGENT-1: No code actions tool (quick fixes, refactors)

**File:** gap in `mcp-server/src/tools.rs`
**Source:** agent-native-reviewer (Critical-1)

When `rust_diagnostics` returns an error with a known fix (missing import, unused variable, wrong type), a human hits "quick fix" and RA applies the change. The agent guesses the fix and edits manually.

**Fix:** Add `rust_code_actions(file_path, line, character)` wrapping `textDocument/codeAction`. Return available actions with their edits as data; let the agent decide which to apply.

---

## AGENT-2: No rename refactoring tool

**File:** gap in `mcp-server/src/tools.rs`
**Source:** agent-native-reviewer (Critical-2)

Renaming a symbol across a codebase is one of the most common refactoring operations. The agent must call `rust_find_references`, read each file, and manually edit each occurrence. Error-prone and slow.

**Fix:** Add `rust_rename(file_path, line, character, new_name)` wrapping `textDocument/rename`. Return the workspace edit (set of file changes) without applying them.

---

## AGENT-3: No document symbols tool (file outline)

**File:** gap in `mcp-server/src/tools.rs`
**Source:** agent-native-reviewer (Critical-3)

No way to get the structure of a single file. `rust_workspace_symbol` searches by name across *all* files. Forces the agent to read entire files and parse them manually.

**Fix:** Add `rust_document_symbols(file_path)` wrapping `textDocument/documentSymbol`. Returns a tree of symbols with kinds and ranges.

---

## Work Log

### 2026-03-24 - Current state

- `mcp-server/src/bootstrap.rs` now uses `libc::getuid()` and Unix socket type checks; added regression tests to keep those bootstrap invariants in place.
- The repository no longer downloads or updates `rust-analyzer` itself. Provisioning is now environment-driven: `RUST_ANALYZER_PATH` first, then `rust-analyzer` on `PATH`, with the flake-exported package intended as the reproducible Nix source of truth.
- The original P1 bootstrap/security gaps are now tracked as resolved by removing the repo-managed download path; the remaining entries in this file are architectural follow-ups rather than the original critical defect class.
