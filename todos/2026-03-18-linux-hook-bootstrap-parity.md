---
status: pending
priority: p2
issue_id: "REV-006"
tags: [hooks, bootstrap, linux, systemd, claude-code]
dependencies: []
---

# Bring Claude hook bootstrap behavior back in sync with runtime bootstrap

The Claude session-start hook currently implements its own bootstrap flow, but that shell copy has drifted from the Rust runtime bootstrap logic. The result is a Linux gap and inconsistent behavior whenever a managed user service should be reused.

## Problem Statement

The repository supports launchd and systemd at setup time, but the Claude hook only knows about launchd. That means a Linux Claude session cannot reuse an installed `lspmux.service` through the hook path, and the direct-start fallback is still inconsistent with the Rust runtime. This undermines the project goal of predictable single-instance sharing per worktree.

## Findings

- `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh:33-44` only attempts `launchctl bootstrap`; there is no `systemctl --user start lspmux.service` branch.
- `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh:51-52` starts `lspmux server` directly without `--config`, unlike `mcp-server/src/bootstrap.rs:253-258`.
- `mcp-server/src/bootstrap.rs:218-246` already contains the correct platform split for launchd vs systemd.
- `docs/hosts/claude-code.md` says hooks are optional optimization only, which means there is no need for the hook to maintain a second full bootstrap implementation.
- Existing review notes already captured the missing `--config` flag, but the Linux/systemd mismatch is an additional behavior gap.

## Proposed Solutions

### Option 1: Reduce the hook to status + guidance (recommended)

**Approach:** Remove service-start logic from `session-start.sh`. Let the hook emit a status/system message only, and rely on the Rust MCP bootstrap path as the single implementation of service startup/reuse policy.

**Pros:**
- Eliminates logic drift between shell and Rust
- Fixes Linux parity by deleting the duplicate bootstrap path
- Simplifies maintenance whenever bootstrap rules change

**Cons:**
- Session-start feedback becomes slightly less proactive
- Claude startup no longer tries to heal service state before MCP startup

**Effort:** 1-2 hours

**Risk:** Low

---

### Option 2: Port the Rust bootstrap logic exactly into shell

**Approach:** Keep hook-level startup, but add systemd support and explicit `--config` handling so the shell path mirrors `bootstrap.rs`.

**Pros:**
- Preserves proactive startup behavior in Claude sessions
- Minimal behavior change from a user perspective

**Cons:**
- Still duplicates policy in two languages
- Future changes will drift again unless rigorously tested

**Effort:** 2-4 hours

**Risk:** Medium

---

### Option 3: Move bootstrap into a shared helper executable

**Approach:** Factor bootstrap into a library-exposed or helper CLI command that both the MCP server and shell hook can call.

**Pros:**
- One implementation, reusable from both entry points
- Leaves room for proactive hook behavior

**Cons:**
- More moving parts than Option 1
- Harder to justify for a small codebase if the hook is optional

**Effort:** 4-6 hours

**Risk:** Medium

## Recommended Action

**To be filled during triage.** Preferred direction: keep `session-start.sh` status-only. The bootstrap parity issue has already been resolved in the hook itself; the remaining work is doc/test alignment so the status-only contract stays explicit and does not drift back toward service management.

## Technical Details

**Affected files:**
- `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh`
- `docs/hosts/claude-code.md`
- Possibly `tests/test-hooks.sh` if behavior changes are validated there

**Related components:**
- `mcp-server/src/bootstrap.rs`
- `setup` (`core` deployment for launchd/systemd)

**Database changes (if any):**
- Migration needed? No
- New columns/tables? No

## Resources

- **Related review notes:** `todos/review-2026-03-18-p2-important.md` (`ARCH-2`, `PAT-3`)
- **Reference implementation:** `mcp-server/src/bootstrap.rs`
- **Docs:** `docs/hosts/claude-code.md`

## Acceptance Criteria

- [ ] Claude session-start behavior works on both macOS and Linux
- [ ] Direct-start fallback uses the same config path as the Rust runtime
- [ ] There is only one authoritative bootstrap policy definition
- [ ] Hook tests cover the chosen behavior path
- [ ] Documentation clearly states whether hooks start services or only report status

## Work Log

### 2026-03-24 - Hook bootstrap parity reduced to status-only

**By:** Codex

**Actions:**
- Verified `session-start.sh` no longer starts services directly.
- Confirmed the remaining hook behavior is informational JSON on stdout only.
- Updated Claude-facing docs and tests to reflect the status-only contract.

**Learnings:**
- The original Linux/systemd parity gap is no longer a live bootstrap-path bug in the hook.
- The remaining maintenance risk is documentation drift, not duplicate bootstrap logic.

### 2026-03-18 - Initial discovery

**By:** Codex

**Actions:**
- Compared the shell hook bootstrap path with `bootstrap.rs`
- Verified that Linux setup exists while the hook remains launchd-only
- Confirmed the direct-start path is still missing `--config`
- Cross-checked host docs indicating hooks are optional optimization only

**Learnings:**
- The simplest fix is likely deletion of duplicate bootstrap logic, not expansion of the shell copy
- Linux support is already conceptually present in the project, so the hook mismatch is easy to miss in macOS-only testing

---

## Notes

- If the hook is reduced to status-only, consider emitting the runtime bootstrap mode and recent health snapshot once the MCP server is up so users still get useful startup feedback.
