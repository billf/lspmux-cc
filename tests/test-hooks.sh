#!/usr/bin/env bash
set -euo pipefail

# Test that Phase 2 hook files are properly structured and valid.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PLUGIN_DIR="${SCRIPT_DIR}/plugins/lspmux-rust-cc"
PASS=0
FAIL=0

pass() { echo "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $*" >&2; FAIL=$((FAIL + 1)); }

echo "=== Phase 2 Hooks Tests ==="

# --- File existence ---
echo "-- File existence --"
for f in hooks/hooks.json hooks/scripts/session-start.sh hooks/scripts/post-file-edit.sh; do
    if [ -f "${PLUGIN_DIR}/${f}" ]; then
        pass "${f} exists"
    else
        fail "${f} missing"
    fi
done

# --- Executable bits ---
echo "-- Executable permissions --"
for f in hooks/scripts/session-start.sh hooks/scripts/post-file-edit.sh; do
    if [ -x "${PLUGIN_DIR}/${f}" ]; then
        pass "${f} is executable"
    else
        fail "${f} is not executable"
    fi
done

# --- JSON validity ---
echo "-- JSON validity --"
if jq . "${PLUGIN_DIR}/hooks/hooks.json" >/dev/null 2>&1; then
    pass "hooks.json is valid JSON"
else
    fail "hooks.json is invalid JSON"
fi

# --- hooks.json structure ---
echo "-- hooks.json structure --"
HOOKS_JSON="${PLUGIN_DIR}/hooks/hooks.json"

if jq -e '.hooks.SessionStart' "${HOOKS_JSON}" >/dev/null 2>&1; then
    pass "hooks.json has SessionStart hook"
else
    fail "hooks.json missing SessionStart hook"
fi

if jq -e '.hooks.PostToolUse' "${HOOKS_JSON}" >/dev/null 2>&1; then
    pass "hooks.json has PostToolUse hook"
else
    fail "hooks.json missing PostToolUse hook"
fi

# --- post-file-edit.sh handles missing input gracefully ---
echo "-- Graceful handling --"
if echo '{}' | bash "${PLUGIN_DIR}/hooks/scripts/post-file-edit.sh" 2>/dev/null; then
    pass "post-file-edit.sh handles empty input"
else
    fail "post-file-edit.sh fails on empty input"
fi

if echo '{"file_path": "src/main.py"}' | bash "${PLUGIN_DIR}/hooks/scripts/post-file-edit.sh" 2>/dev/null; then
    pass "post-file-edit.sh handles non-Rust file"
else
    fail "post-file-edit.sh fails on non-Rust file"
fi

# --- Summary ---
echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
