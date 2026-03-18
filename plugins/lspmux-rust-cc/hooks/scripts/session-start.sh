#!/usr/bin/env bash
set -euo pipefail

# Report shared-service status at session start.
# Bootstrap decisions live in the Rust MCP runtime.

LSPMUX_BIN="${LSPMUX_PATH:-${CARGO_HOME:-$HOME/.cargo}/bin/lspmux}"
BOOTSTRAP_MODE="${LSPMUX_BOOTSTRAP:-auto}"
WS="${WORKSPACE_ROOT:-(not set)}"

check_running() {
    "${LSPMUX_BIN}" status >/dev/null 2>&1
}

if ! [ -x "${LSPMUX_BIN}" ]; then
    echo '{"systemMessage": "WARNING: lspmux not installed. Run setup script."}' || true
    exit 2
fi

if [ "${BOOTSTRAP_MODE}" = "off" ]; then
    printf '{"systemMessage": "lspmux bootstrap disabled (LSPMUX_BOOTSTRAP=off). Workspace: %s"}' "${WS}"
    echo
    exit 0
fi

if check_running; then
    printf '{"systemMessage": "Shared lspmux rust-analyzer service is already running. Workspace: %s\\nCheck rust_server_status after MCP startup for bootstrap mode and readiness."}' "${WS}"
    echo
    exit 0
fi

if [ "${BOOTSTRAP_MODE}" = "require" ]; then
    echo '{"systemMessage": "WARNING: shared lspmux service is not running and bootstrap is required. Run ./setup core, then use rust_server_status after MCP startup to verify readiness."}' || true
    exit 2
fi

printf '{"systemMessage": "Shared lspmux rust-analyzer service is not running yet. Workspace: %s\\nThe MCP runtime will handle bootstrap on first use. Check rust_server_status after startup for bootstrap mode and readiness."}' "${WS}"
echo
exit 0
