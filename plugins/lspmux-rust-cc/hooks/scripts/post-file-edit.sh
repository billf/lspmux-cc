#!/usr/bin/env bash
set -euo pipefail

# Notify lspmux of file changes after Write/Edit tool use.
# Reads tool input from stdin to extract file_path.
# Only triggers sync for Rust-related files.

if [ -n "${LSPMUX_PATH:-}" ] && [ -x "${LSPMUX_PATH}" ]; then
    LSPMUX_BIN="${LSPMUX_PATH}"
elif command -v lspmux >/dev/null 2>&1; then
    LSPMUX_BIN="$(command -v lspmux)"
else
    LSPMUX_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/lspmux"
fi
HOOK_NAME="post-file-edit"

log_msg() {
    printf '[lspmux-cc:%s] %s\n' "${HOOK_NAME}" "$*" >&2
}

# Read tool input from stdin
INPUT="$(cat)"

# Extract file_path from JSON input
FILE_PATH="$(
    printf '%s\n' "${INPUT}" \
        | jq -r '.file_path // .filePath // ""' 2>/dev/null
)" || {
    log_msg "warning: failed to parse hook input JSON"
    exit 0
}

if [ -z "${FILE_PATH}" ]; then
    exit 0
fi

# Only sync for Rust-related files
case "${FILE_PATH}" in
    *.rs|*/Cargo.toml|*/Cargo.lock)
        if [ -x "${LSPMUX_BIN}" ]; then
            if ! SYNC_OUTPUT="$("${LSPMUX_BIN}" sync 2>&1)"; then
                if [ -n "${SYNC_OUTPUT}" ]; then
                    log_msg "sync failed: ${SYNC_OUTPUT}"
                else
                    log_msg "sync failed with no stderr output"
                fi
            fi
        fi
        ;;
esac

exit 0
