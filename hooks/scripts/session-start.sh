#!/usr/bin/env bash
set -euo pipefail

# Report shared-service status at session start.
# Bootstrap decisions live in the Rust MCP runtime.

if [ -n "${CLAUDE_PLUGIN_ROOT:-}" ] && [ -x "${CLAUDE_PLUGIN_ROOT}/bin/lspmux" ]; then
    LSPMUX_BIN="${CLAUDE_PLUGIN_ROOT}/bin/lspmux"
elif [ -n "${LSPMUX_PATH:-}" ]; then
    if [ -x "${LSPMUX_PATH}" ]; then
        LSPMUX_BIN="${LSPMUX_PATH}"
    else
        printf '%s\n' '{"systemMessage": "WARNING: LSPMUX_PATH is set but not executable: '"${LSPMUX_PATH}"'. Unset it or fix the path."}'
        exit 2
    fi
elif command -v lspmux >/dev/null 2>&1; then
    LSPMUX_BIN="$(command -v lspmux)"
else
    LSPMUX_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/lspmux"
fi
BOOTSTRAP_MODE="${LSPMUX_BOOTSTRAP:-auto}"
WS="${WORKSPACE_ROOT:-(not set)}"

if ! [ -x "${LSPMUX_BIN}" ]; then
    printf '%s\n' '{"systemMessage": "WARNING: lspmux not installed. Run setup script."}'
    exit 2
fi

if [ "${BOOTSTRAP_MODE}" = "off" ]; then
    jq -n --arg ws "${WS}" '{"systemMessage": "lspmux bootstrap disabled (LSPMUX_BOOTSTRAP=off). Workspace: \($ws)"}'
    exit 0
fi

# Canonicalize the requested workspace so we can compare against the daemon's
# instances[].workspaceRoot.path values, which are absolute and (typically)
# already canonical.
WS_CANON=""
if [ "${WS}" != "(not set)" ] && [ -d "${WS}" ]; then
    if ! WS_CANON="$(cd "${WS}" 2>/dev/null && pwd -P)"; then
        WS_CANON=""
    fi
fi

# Probe the daemon. `lspmux status` exits non-zero when the daemon is down;
# `--json` emits {"instances":[...]} otherwise. Defensive parsing throughout —
# unknown schema is treated the same as daemon-down.
# Bound the probe at 3s so a wedged daemon can't hang SessionStart.
# `timeout` is GNU coreutils (`gtimeout` on stock macOS via Homebrew/nix); when
# neither is present, fall back to a perl alarm shim. The alarm timer survives
# `exec`, and SIGALRM's default action kills the child, so the probe stays
# bounded everywhere perl exists (macOS base /usr/bin/perl; most Linux distros).
if command -v timeout >/dev/null 2>&1; then
    STATUS_JSON="$(timeout 3 "${LSPMUX_BIN}" status --json 2>/dev/null || true)"
elif command -v gtimeout >/dev/null 2>&1; then
    STATUS_JSON="$(gtimeout 3 "${LSPMUX_BIN}" status --json 2>/dev/null || true)"
elif command -v perl >/dev/null 2>&1; then
    STATUS_JSON="$(perl -e 'alarm shift; exec @ARGV' 3 "${LSPMUX_BIN}" status --json 2>/dev/null || true)"
else
    # No deadline mechanism available. Skip the probe rather than risk a hang.
    # Downstream treats an empty STATUS_JSON the same as daemon-down, so the
    # hook errs toward "not running"; the user can confirm via rust_server_status.
    STATUS_JSON=""
fi
DAEMON_UP=0
if [ -n "${STATUS_JSON}" ] && printf '%s' "${STATUS_JSON}" | jq -e '.instances' >/dev/null 2>&1; then
    DAEMON_UP=1
fi

if [ "${DAEMON_UP}" -eq 0 ]; then
    if [ "${BOOTSTRAP_MODE}" = "require" ]; then
        printf '%s\n' '{"systemMessage": "WARNING: shared lspmux service is not running and bootstrap is required. Run ./setup core, then use rust_server_status after MCP startup to verify readiness."}'
        exit 2
    fi
    jq -n --arg ws "${WS}" '{"systemMessage": "Shared lspmux rust-analyzer service is not running yet. Workspace: \($ws)\nThe MCP runtime will handle bootstrap on first use. Check rust_server_status after startup for bootstrap mode and readiness."}'
    exit 0
fi

# Find an instance whose workspaceRoot.path matches the canonical workspace.
MATCH_LINE=""
if [ -n "${WS_CANON}" ]; then
    MATCH_LINE="$(printf '%s' "${STATUS_JSON}" \
        | jq -r --arg ws "${WS_CANON}" \
            '.instances[]? | select(.workspaceRoot.path == $ws) | "\(.pid) \(.idleFor // "?")"' \
        | head -n1)"
fi

# Distinct served workspaces, comma-separated, for the mismatch message.
SERVED="$(printf '%s' "${STATUS_JSON}" \
    | jq -r '[.instances[]?.workspaceRoot.path] | unique | join(", ")')"

INSTANCE_COUNT="$(printf '%s' "${STATUS_JSON}" | jq -r '.instances | length')"

if [ -n "${MATCH_LINE}" ]; then
    jq -n --arg ws "${WS}" --arg match "${MATCH_LINE}" --arg total "${INSTANCE_COUNT}" \
        '{"systemMessage": "Shared lspmux rust-analyzer is serving this workspace (pid \($match | split(" ")[0]), idle \($match | split(" ")[1])ms). Daemon hosts \($total) workspace(s). Workspace: \($ws)\nCheck rust_server_status or rust_workspace_registry for more detail."}'
    exit 0
fi

# Daemon is up but not serving this workspace. M2 will fix this by routing
# requests to a per-worktree daemon; until then, surface the mismatch honestly.
if [ -z "${WS_CANON}" ]; then
    jq -n --arg ws "${WS}" --arg served "${SERVED}" \
        '{"systemMessage": "Shared lspmux rust-analyzer is running but no workspace was provided to compare against. Currently serving: \($served). Workspace: \($ws)"}'
    exit 0
fi

jq -n --arg ws "${WS_CANON}" --arg served "${SERVED}" \
    '{"systemMessage": "WARNING: lspmux is running but is NOT yet serving this workspace. Currently serving: \($served). The MCP runtime will spin up a rust-analyzer for \($ws) on first request. Check rust_workspace_registry for the daemon'\''s full instance list."}'
exit 0
