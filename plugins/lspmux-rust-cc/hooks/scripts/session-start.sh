#!/usr/bin/env bash
set -euo pipefail

# Report shared-service status at session start.
# Bootstrap decisions live in the Rust MCP runtime.

if [ -n "${LSPMUX_PATH:-}" ] && [ -x "${LSPMUX_PATH}" ]; then
    LSPMUX_BIN="${LSPMUX_PATH}"
elif command -v lspmux >/dev/null 2>&1; then
    LSPMUX_BIN="$(command -v lspmux)"
else
    LSPMUX_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/lspmux"
fi
BOOTSTRAP_MODE="${LSPMUX_BOOTSTRAP:-auto}"
WS="${WORKSPACE_ROOT:-(not set)}"

check_running() {
    "${LSPMUX_BIN}" status >/dev/null 2>&1
}

if ! [ -x "${LSPMUX_BIN}" ]; then
    printf '%s\n' '{"systemMessage": "WARNING: lspmux not installed. Run setup script."}'
    exit 2
fi

if [ "${BOOTSTRAP_MODE}" = "off" ]; then
    jq -n --arg ws "${WS}" '{"systemMessage": "lspmux bootstrap disabled (LSPMUX_BOOTSTRAP=off). Workspace: \($ws)"}'
    exit 0
fi

if check_running; then
    jq -n --arg ws "${WS}" '{"systemMessage": "Shared lspmux rust-analyzer service is already running. Workspace: \($ws)\nCheck rust_server_status after MCP startup for bootstrap mode and readiness."}'
    exit 0
fi

if [ "${BOOTSTRAP_MODE}" = "require" ]; then
    printf '%s\n' '{"systemMessage": "WARNING: shared lspmux service is not running and bootstrap is required. Run ./setup core, then use rust_server_status after MCP startup to verify readiness."}'
    exit 2
fi

jq -n --arg ws "${WS}" '{"systemMessage": "Shared lspmux rust-analyzer service is not running yet. Workspace: \($ws)\nThe MCP runtime will handle bootstrap on first use. Check rust_server_status after startup for bootstrap mode and readiness."}'
exit 0
