#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WRAPPER="${SCRIPT_DIR}/plugins/lspmux-rust-cc/bin/lspmux-cc-mcp"

TEST_DIR="$(mktemp -d)"
trap 'rm -rf "${TEST_DIR}"' EXIT

HOME_DIR="${TEST_DIR}/home"
mkdir -p "${HOME_DIR}/.cargo/bin"

PASS=0
FAIL=0
pass() { echo "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $*" >&2; FAIL=$((FAIL + 1)); }

cat > "${HOME_DIR}/.cargo/bin/lspmux-cc-mcp" <<'EOF'
#!/usr/bin/env bash
printf '%s|%s|%s\n' \
    "${LSPMUX_CLIENT_KIND:-}" \
    "${LSPMUX_CLIENT_HOST:-}" \
    "${LSPMUX_SESSION_ID:-}"
EOF
chmod +x "${HOME_DIR}/.cargo/bin/lspmux-cc-mcp"

run_wrapper() {
    env -i PATH="${PATH}" HOME="${HOME_DIR}" "$@"
}

echo "=== MCP Wrapper Tests ==="

echo "-- Default env values --"
default_output="$(run_wrapper "${WRAPPER}")"
IFS='|' read -r default_kind default_host default_session <<<"${default_output}"

if [ "${default_kind}" = "claude_mcp" ]; then
    pass "default kind is claude_mcp"
else
    fail "expected default kind claude_mcp, got ${default_kind}"
fi

if [ "${default_host}" = "claude" ]; then
    pass "default host is claude"
else
    fail "expected default host claude, got ${default_host}"
fi

if [[ "${default_session}" =~ ^claude-mcp-[0-9]+-[0-9]+$ ]]; then
    pass "default session id has claude-mcp prefix"
else
    fail "expected default session id with claude-mcp prefix, got ${default_session}"
fi

echo "-- Caller-provided env values --"
custom_output="$(
    env -i \
        PATH="${PATH}" \
        HOME="${HOME_DIR}" \
        LSPMUX_CLIENT_KIND=custom_kind \
        LSPMUX_CLIENT_HOST=custom_host \
        LSPMUX_SESSION_ID=custom-session \
        "${WRAPPER}"
)"
IFS='|' read -r custom_kind custom_host custom_session <<<"${custom_output}"

if [ "${custom_kind}" = "custom_kind" ]; then
    pass "caller-provided kind is preserved"
else
    fail "expected caller-provided kind to be preserved, got ${custom_kind}"
fi

if [ "${custom_host}" = "custom_host" ]; then
    pass "caller-provided host is preserved"
else
    fail "expected caller-provided host to be preserved, got ${custom_host}"
fi

if [ "${custom_session}" = "custom-session" ]; then
    pass "caller-provided session is preserved"
else
    fail "expected caller-provided session to be preserved, got ${custom_session}"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
