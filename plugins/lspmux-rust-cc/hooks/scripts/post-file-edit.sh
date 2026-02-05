#!/usr/bin/env bash
set -euo pipefail

# Notify lspmux of file changes after Write/Edit tool use.
# Reads tool input from stdin to extract file_path.
# Only triggers sync for Rust-related files.

LSPMUX_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/lspmux"

# Read tool input from stdin
INPUT="$(cat)"

# Extract file_path from JSON input
FILE_PATH="$(echo "${INPUT}" | jq -r '.file_path // .filePath // ""' 2>/dev/null)" || true

if [ -z "${FILE_PATH}" ]; then
    exit 0
fi

# Only sync for Rust-related files
case "${FILE_PATH}" in
    *.rs|*/Cargo.toml|*/Cargo.lock)
        if [ -x "${LSPMUX_BIN}" ]; then
            "${LSPMUX_BIN}" sync 2>/dev/null || true
        fi
        ;;
esac

exit 0
