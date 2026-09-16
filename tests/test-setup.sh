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
for f in setup config/lspmux.toml \
         launchd/com.lspmux.server.plist \
         systemd/lspmux.service \
         .claude-plugin/plugin.json \
         bin/lspmux bin/rust-analyzer \
         .claude-plugin/marketplace.json \
         .mcp.json .lsp.json \
         docs/hosts/claude-code.md docs/hosts/codex.md docs/hosts/generic-mcp.md; do
    if [ -f "${SCRIPT_DIR}/${f}" ]; then
        pass "${f} exists"
    else
        fail "${f} missing"
    fi
done

# --- Executable bits ---
echo "-- Executable permissions --"
for f in setup bin/lspmux \
         bin/rust-analyzer; do
    if [ -x "${SCRIPT_DIR}/${f}" ]; then
        pass "${f} is executable"
    else
        fail "${f} is not executable"
    fi
done

# --- JSON validity ---
echo "-- JSON validity --"
for f in .claude-plugin/plugin.json \
         .claude-plugin/marketplace.json \
         .mcp.json .lsp.json; do
    if jq . "${SCRIPT_DIR}/${f}" >/dev/null 2>&1; then
        pass "${f} is valid JSON"
    else
        fail "${f} is invalid JSON"
    fi
done

# --- TOML validity ---
echo "-- TOML validity --"
expected_listen="listen = \"\${SOCKET_PATH}\""
expected_connect="connect = \"\${SOCKET_PATH}\""
if python3 -c "import importlib.util, pathlib, sys; p=pathlib.Path('${SCRIPT_DIR}/config/lspmux.toml'); mod = 'tomllib' if importlib.util.find_spec('tomllib') else 'tomli' if importlib.util.find_spec('tomli') else None; sys.exit(2 if mod is None else 0)" 2>/dev/null; then
    if python3 -c "import importlib.util, pathlib; p=pathlib.Path('${SCRIPT_DIR}/config/lspmux.toml'); mod = 'tomllib' if importlib.util.find_spec('tomllib') else 'tomli'; parser = __import__(mod); parser.load(open(p, 'rb'))" 2>/dev/null; then
        pass "config/lspmux.toml is valid TOML"
    else
        fail "config/lspmux.toml is invalid TOML"
    fi
elif grep -q '^instance_timeout = 300$' "${SCRIPT_DIR}/config/lspmux.toml" \
    && grep -q '^gc_interval = 10$' "${SCRIPT_DIR}/config/lspmux.toml" \
    && grep -Fqx "${expected_listen}" "${SCRIPT_DIR}/config/lspmux.toml" \
    && grep -Fqx "${expected_connect}" "${SCRIPT_DIR}/config/lspmux.toml"; then
    pass "config/lspmux.toml is valid TOML"
else
    fail "config/lspmux.toml is invalid TOML"
fi

# --- plist validity ---
echo "-- plist validity --"
if plutil -lint "${SCRIPT_DIR}/launchd/com.lspmux.server.plist" >/dev/null 2>&1; then
    pass "launchd/com.lspmux.server.plist is valid plist"
else
    fail "launchd/com.lspmux.server.plist is invalid plist"
fi

# --- No downloader fallback ---
echo "-- Rust analyzer provisioning contract --"
if ! grep -qE 'update-rust-analyzer|lspmux-rust-analyzer/current|result-rust-analyzer-nightly' \
    "${SCRIPT_DIR}/setup" \
    "${SCRIPT_DIR}/bin/rust-analyzer" \
    "${SCRIPT_DIR}/docs/hosts/codex.md" \
    "${SCRIPT_DIR}/docs/hosts/generic-mcp.md"; then
    pass "setup and wrappers no longer reference repo-managed rust-analyzer downloads"
else
    fail "setup or wrappers still reference repo-managed rust-analyzer downloads"
fi

# --- lspmux compatibility contract ---
echo "-- lspmux compatibility contract --"
if grep -Fq 'cargo install --locked lspmux --version 0.3.0' "${SCRIPT_DIR}/setup" \
    && grep -Fq 'validate_lspmux_version "${lspmux_binary}"' "${SCRIPT_DIR}/setup" \
    && grep -Fq 'lspmux server` without `--config`' "${SCRIPT_DIR}/mcp-server/src/bootstrap.rs"; then
    pass "setup pins and bootstrap validates the lspmux 0.3.x contract"
else
    fail "lspmux install or runtime compatibility contract is missing"
fi

# --- lspmux version validator behavior ---
# The grep above proves the validate call site exists; this matrix proves the
# validator enforces the range by executing setup's function against stubs.
echo "-- lspmux version validator behavior --"
LSPMUX_VALIDATOR_TMP="$(mktemp -d)"
trap 'rm -rf "${LSPMUX_VALIDATOR_TMP}"' EXIT
awk '/^validate_lspmux_version\(\) \{/,/^\}$/' "${SCRIPT_DIR}/setup" > "${LSPMUX_VALIDATOR_TMP}/validator.sh"
info() { :; }
die() { echo "ERROR: $*" >&2; exit 1; }
# shellcheck source=/dev/null
source "${LSPMUX_VALIDATOR_TMP}/validator.sh"
check_lspmux_version() {
    local expected="$1" output="$2"
    local fake="${LSPMUX_VALIDATOR_TMP}/fake-lspmux" actual
    if [ "${output}" = "__FAIL__" ]; then
        printf '#!/bin/sh\nexit 1\n' > "${fake}"
    else
        printf '#!/bin/sh\necho "%s"\n' "${output}" > "${fake}"
    fi
    chmod +x "${fake}"
    if ( validate_lspmux_version "${fake}" >/dev/null 2>&1 ); then
        actual="accept"
    else
        actual="reject"
    fi
    if [ "${actual}" = "${expected}" ]; then
        pass "validate_lspmux_version ${actual}s '${output}'"
    else
        fail "validate_lspmux_version ${actual}s '${output}' (expected ${expected})"
    fi
}
check_lspmux_version accept "lspmux 0.3.0"
check_lspmux_version accept "lspmux 0.3.7"
check_lspmux_version accept "lspmux 0.3.1-alpha"
check_lspmux_version accept "lspmux 0.3.1+build"
check_lspmux_version reject "lspmux 0.2.9"
check_lspmux_version reject "lspmux 0.4.0"
check_lspmux_version reject "lspmux 0.3.0-alpha"
check_lspmux_version reject "lspmux 0.3.0-alpha.01"
check_lspmux_version reject "not a version"
check_lspmux_version reject "lspmux version 0.3.0"
check_lspmux_version reject "__FAIL__"

# --- plugin.json structure ---
echo "-- plugin.json structure --"
PLUGIN_JSON="${SCRIPT_DIR}/.claude-plugin/plugin.json"
if jq -e '.name' "${PLUGIN_JSON}" >/dev/null 2>&1; then
    pass "plugin.json has name field"
else
    fail "plugin.json missing name field"
fi

# --- .lsp.json structure ---
echo "-- .lsp.json structure --"
LSP_JSON="${SCRIPT_DIR}/.lsp.json"
if jq -e '.["rust-analyzer"].command' "${LSP_JSON}" >/dev/null 2>&1; then
    pass ".lsp.json has rust-analyzer server"
else
    fail ".lsp.json missing rust-analyzer server"
fi

# --- Summary ---
echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
