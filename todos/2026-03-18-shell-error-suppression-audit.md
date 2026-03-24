---
status: pending
priority: p3
issue_id: "REV-009"
tags: [observability, shell, error-handling, hooks]
dependencies: []
---

# Audit and fix error suppression patterns in shell scripts

## Problem Statement

Multiple shell scripts redirect stderr to `/dev/null` or append `|| true`, making failures invisible. This is distinct from REV-006 (which covers bootstrap logic drift) and PAT-1 (which covers error message format inconsistency). This TODO specifically addresses error *visibility*: operations that fail silently with no trace.

Operators can't tell when `lspmux sync` fails, when `launchctl bootstrap` fails, or when hooks encounter unexpected state. The system appears healthy even when parts of it aren't working.

## Findings

- `plugins/lspmux-rust-cc/hooks/scripts/post-file-edit.sh:24`: `"${LSPMUX_BIN}" sync 2>/dev/null || true` — sync failures are completely silent. If lspmux is down, every file edit hook fires and fails without evidence.
- `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh:37`: `launchctl bootstrap "gui/$(id -u)" "${PLIST}" 2>/dev/null || true` — bootstrap failures (wrong label, corrupt plist, permission denied) are invisible.
- `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh:16`: `echo '...' || true` — benign but confusing; `echo` to stdout doesn't fail, and the `|| true` suggests someone was worried it might.
- `plugins/lspmux-rust-cc/hooks/scripts/post-file-edit.sh:14`: `FILE_PATH="$(echo "${INPUT}" | jq -r '...' 2>/dev/null)" || true` — jq parse failures suppressed; if the hook input format changes, this silently produces empty paths.
- The core constraint is that stdout is the MCP/Claude JSON protocol channel. One stray `echo` breaks framing. stderr is the only safe output channel, but it's being suppressed.

## Proposed Solutions

### Option 1: Capture stderr before suppressing (recommended)

**Approach:** Replace `2>/dev/null || true` with a pattern that captures stderr, logs it, then continues:

```bash
set +e
output=$("${LSPMUX_BIN}" sync 2>&1)
rc=$?
set -e
if [ ${rc} -ne 0 ]; then
    log_msg "sync failed (rc=${rc}): ${output}"
fi
```

Add a `log_msg()` helper that writes to stderr + optional sidecar file via `$LSPMUX_LOG_DIR`:

```bash
HOOK_NAME="$(basename "$0" .sh)"
LOG_DIR="${LSPMUX_LOG_DIR:-}"

log_msg() {
    if [ -n "${LOG_DIR}" ]; then
        mkdir -p "${LOG_DIR}"
        printf '[%s] [%s] %s\n' "$(date -Iseconds)" "${HOOK_NAME}" "$*" >> "${LOG_DIR}/hooks.log"
    fi
    printf '[lspmux-cc:%s] %s\n' "${HOOK_NAME}" "$*" >&2
}
```

**Pros:**
- Errors become visible in stderr and optional log file
- Exit codes preserved for debugging
- No change to MCP protocol (stdout untouched)

**Cons:**
- Slightly more verbose scripts
- Sidecar log file needs eventual rotation

**Effort:** 1-2 hours

**Risk:** Low

---

### Option 2: Remove suppression entirely

**Approach:** Let errors propagate to stderr naturally. Remove `2>/dev/null` and `|| true` where the calling context can tolerate visible errors.

**Pros:**
- Simplest change
- Errors immediately visible

**Cons:**
- Some callers (Claude Code hook runner) may treat stderr output as warnings, cluttering the UI
- Hard failures in hooks (`set -e` + no `|| true`) could kill the hook process

**Effort:** 30 minutes

**Risk:** Medium (behavior change visible to users)

---

### Option 3: Log to sidecar file only (no stderr)

**Approach:** Redirect suppressed stderr to `$LSPMUX_LOG_DIR/hooks.log` instead of `/dev/null`. Don't write to stderr.

**Pros:**
- Silent to the user by default
- Errors available for post-mortem debugging

**Cons:**
- Requires `LSPMUX_LOG_DIR` to be set (or a default)
- Less discoverable than stderr output

**Effort:** 1 hour

**Risk:** Low

## Recommended Action

Implement Option 1. The `log_msg()` helper pattern gives visibility without disrupting the user experience. The previously identified suppression sites in `post-file-edit.sh` are now handled; keep the stderr-only contract in place and treat any future shell suppression as a regression.

## Technical Details

**Affected files:**
- `plugins/lspmux-rust-cc/hooks/scripts/post-file-edit.sh` — lines 14, 24
- `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh` — lines 16, 37

**Related components:**
- `setup` script's `die()`/`warn()` functions (for format consistency)
- REV-006 (bootstrap logic drift; this TODO is about error visibility, not logic)
- PAT-1 (error message format inconsistency)
- REV-004 (observability roadmap; this is a quick shell-side win)

## Resources

- **Related review:** `todos/review-2026-03-18-p3-cleanup.md` (PAT-1)
- **Related todo:** `todos/2026-03-18-linux-hook-bootstrap-parity.md` (REV-006)
- **Related todo:** `todos/2026-03-18-observability-and-client-attribution.md` (REV-004)

## Acceptance Criteria

- [ ] No `2>/dev/null || true` patterns remain in hook scripts without error capture
- [ ] All hook errors are logged to stderr with `[lspmux-cc:<hook>]` prefix
- [ ] Optional sidecar logging to `$LSPMUX_LOG_DIR/hooks.log` when env var is set
- [ ] `just shellcheck` still passes
- [ ] Hook behavior unchanged when operations succeed (no new output on happy path)

## Work Log

### 2026-03-24 - Hook error visibility now surfaced on stderr

**By:** Codex

**Actions:**
- Replaced the file-edit hook's silent JSON-parse and sync-failure paths with stderr logging.
- Removed pointless `|| true` suffixes from the session-start hook's JSON emission.
- Updated Claude docs and hook tests to assert the no-silent-failure contract.

**Learnings:**
- The remaining risk is regression, not missing implementation.
- stderr-only diagnostics preserve the hook protocol while making failures visible.

### 2026-03-18 - Initial discovery

**By:** Claude Code (deep review follow-up)

**Actions:**
- Audited all shell scripts for `2>/dev/null` and `|| true` patterns
- Identified 4 suppression sites across 2 hook scripts
- Researched shell error capture patterns that preserve exit codes and respect stdout/MCP constraints

**Learnings:**
- stdout is sacred (MCP JSON protocol); stderr is the only safe error channel
- The `set +e; output=$(...); rc=$?; set -e` pattern is robust and standard
- A shared `log_msg()` helper should be extracted to avoid duplicating the pattern

---

## Notes

- The `log_msg()` helper could live in a shared `lib.sh` sourced by both hooks, but that adds a dependency. For two scripts, inline is fine.
- Consider adding `LSPMUX_LOG_DIR` to the documented configuration env vars after this lands.
