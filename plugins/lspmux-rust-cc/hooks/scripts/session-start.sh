#!/usr/bin/env bash
set -euo pipefail

# Ensure lspmux server is running at session start.
# Outputs a systemMessage if successful.

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

# Already running?
if check_running; then
    printf '{"systemMessage": "Shared lspmux rust-analyzer service is running. Workspace: %s\\nTools: rust_diagnostics, rust_hover, rust_goto_definition, rust_find_references, rust_workspace_symbol, rust_server_status\\nCoordinates: inputs are 0-based; output locations are 1-based."}' "${WS}"
    echo
    exit 0
fi

# Try launchctl bootstrap
LABEL="com.lspmux.server"
PLIST="${HOME}/Library/LaunchAgents/${LABEL}.plist"
if [ -f "${PLIST}" ]; then
    launchctl bootstrap "gui/$(id -u)" "${PLIST}" 2>/dev/null || true
    sleep 2
    if check_running; then
        printf '{"systemMessage": "Shared lspmux rust-analyzer service started via launchd. Workspace: %s\\nTools: rust_diagnostics, rust_hover, rust_goto_definition, rust_find_references, rust_workspace_symbol, rust_server_status\\nCoordinates: inputs are 0-based; output locations are 1-based."}' "${WS}"
        echo
        exit 0
    fi
fi

if [ "${BOOTSTRAP_MODE}" = "require" ]; then
    echo '{"systemMessage": "WARNING: lspmux service was required but is not available. Run ./setup core or set LSPMUX_BOOTSTRAP=auto."}' || true
    exit 2
fi

# Last resort: start directly
"${LSPMUX_BIN}" server &
disown
sleep 2
if check_running; then
    printf '{"systemMessage": "Shared lspmux rust-analyzer service started directly. Workspace: %s\\nTools: rust_diagnostics, rust_hover, rust_goto_definition, rust_find_references, rust_workspace_symbol, rust_server_status\\nCoordinates: inputs are 0-based; output locations are 1-based."}' "${WS}"
    echo
    exit 0
fi

echo '{"systemMessage": "WARNING: Failed to start lspmux server. Check logs."}' || true
exit 2
