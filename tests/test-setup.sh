#!/usr/bin/env bash
set -euo pipefail

# Test that Phase 1 files are properly structured and valid.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PASS=0
FAIL=0

pass() { echo "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $*" >&2; FAIL=$((FAIL + 1)); }

echo "=== Phase 1 Setup Tests ==="

# --- File existence ---
echo "-- File existence --"
for f in setup bin/update-rust-analyzer config/lspmux.toml \
         launchd/com.lspmux.server.plist launchd/com.rust-analyzer.update.plist \
         plugins/lspmux-rust-cc/.claude-plugin/plugin.json \
         plugins/lspmux-rust-cc/bin/lspmux plugins/lspmux-rust-cc/bin/rust-analyzer \
         .claude-plugin/marketplace.json; do
    if [ -f "${SCRIPT_DIR}/${f}" ]; then
        pass "${f} exists"
    else
        fail "${f} missing"
    fi
done

# --- Executable bits ---
echo "-- Executable permissions --"
for f in setup bin/update-rust-analyzer plugins/lspmux-rust-cc/bin/lspmux \
         plugins/lspmux-rust-cc/bin/rust-analyzer; do
    if [ -x "${SCRIPT_DIR}/${f}" ]; then
        pass "${f} is executable"
    else
        fail "${f} is not executable"
    fi
done

# --- JSON validity ---
echo "-- JSON validity --"
for f in plugins/lspmux-rust-cc/.claude-plugin/plugin.json \
         .claude-plugin/marketplace.json; do
    if jq . "${SCRIPT_DIR}/${f}" >/dev/null 2>&1; then
        pass "${f} is valid JSON"
    else
        fail "${f} is invalid JSON"
    fi
done

# --- TOML validity ---
echo "-- TOML validity --"
if python3 -c "import tomllib; tomllib.load(open('${SCRIPT_DIR}/config/lspmux.toml', 'rb'))" 2>/dev/null; then
    pass "config/lspmux.toml is valid TOML"
else
    fail "config/lspmux.toml is invalid TOML"
fi

# --- plist validity ---
echo "-- plist validity --"
for f in launchd/com.lspmux.server.plist launchd/com.rust-analyzer.update.plist; do
    if plutil -lint "${SCRIPT_DIR}/${f}" >/dev/null 2>&1; then
        pass "${f} is valid plist"
    else
        fail "${f} is invalid plist"
    fi
done

# --- plugin.json structure ---
echo "-- plugin.json structure --"
PLUGIN_JSON="${SCRIPT_DIR}/plugins/lspmux-rust-cc/.claude-plugin/plugin.json"
if jq -e '.lspServers["rust-analyzer"].command' "${PLUGIN_JSON}" >/dev/null 2>&1; then
    pass "plugin.json has rust-analyzer lspServer"
else
    fail "plugin.json missing rust-analyzer lspServer"
fi

# --- Summary ---
echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
