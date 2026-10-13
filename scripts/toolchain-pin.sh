#!/usr/bin/env bash
set -euo pipefail

# Shared by the interactive guard and the flake check. Only quoted TOML
# channels are valid; an absent or bare channel produces an empty result.
grep -E '^[[:space:]]*channel[[:space:]]*=' "$1" | head -n 1 | \
    sed -nE -e 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^" ]+)"[[:space:]]*(#.*)?$/\1/p' \
    -e "s/^[[:space:]]*channel[[:space:]]*=[[:space:]]*'([^' ]+)'[[:space:]]*(#.*)?$/\1/p"
