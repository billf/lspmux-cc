#!/usr/bin/env bash
set -euo pipefail

# Test that Phase 2 hook files are properly structured and valid.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PLUGIN_DIR="${SCRIPT_DIR}/plugins/lspmux-rust-cc"
SESSION_START="${PLUGIN_DIR}/hooks/scripts/session-start.sh"
POST_FILE_EDIT="${PLUGIN_DIR}/hooks/scripts/post-file-edit.sh"
RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/lspmux-hooks.XXXXXX")"
PASS=0
FAIL=0
RUN_RC=0
RUN_STDOUT_FILE=""
RUN_STDERR_FILE=""

trap 'rm -rf "${RUN_DIR}"' EXIT

pass() { echo "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $*" >&2; FAIL=$((FAIL + 1)); }

run_hook() {
    local stdin_data="$1"
    shift

    RUN_STDOUT_FILE="$(mktemp "${RUN_DIR}/stdout.XXXXXX")"
    RUN_STDERR_FILE="$(mktemp "${RUN_DIR}/stderr.XXXXXX")"

    set +e
    printf '%s' "${stdin_data}" | "$@" >"${RUN_STDOUT_FILE}" 2>"${RUN_STDERR_FILE}"
    RUN_RC=$?
    set -e
}

assert_stdout_empty() {
    if [ ! -s "${RUN_STDOUT_FILE}" ]; then
        return 0
    fi

    fail "stdout was not empty"
    sed -n '1,20p' "${RUN_STDOUT_FILE}" >&2
    return 1
}

assert_stderr_empty() {
    if [ ! -s "${RUN_STDERR_FILE}" ]; then
        return 0
    fi

    fail "stderr was not empty"
    sed -n '1,20p' "${RUN_STDERR_FILE}" >&2
    return 1
}

assert_stdout_json() {
    if jq -e . "${RUN_STDOUT_FILE}" >/dev/null 2>&1; then
        return 0
    fi

    fail "stdout was not valid JSON"
    sed -n '1,20p' "${RUN_STDOUT_FILE}" >&2
    return 1
}

assert_stdout_contains() {
    local needle="$1"
    if grep -Fq "${needle}" "${RUN_STDOUT_FILE}"; then
        return 0
    fi

    fail "stdout did not contain: ${needle}"
    sed -n '1,20p' "${RUN_STDOUT_FILE}" >&2
    return 1
}

assert_stderr_contains() {
    local needle="$1"
    if grep -Fq "${needle}" "${RUN_STDERR_FILE}"; then
        return 0
    fi

    fail "stderr did not contain: ${needle}"
    sed -n '1,20p' "${RUN_STDERR_FILE}" >&2
    return 1
}

make_lspmux_stub() {
    local stub="${RUN_DIR}/lspmux-stub"
    cat >"${stub}" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
    status)
        exit 1
        ;;
    sync)
        printf 'simulated sync failure\n' >&2
        exit 1
        ;;
    *)
        exit 0
        ;;
esac
EOF
    chmod +x "${stub}"
    printf '%s\n' "${stub}"
}

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
LSPMUX_STUB="$(make_lspmux_stub)"

run_hook 'not-json' env LSPMUX_PATH="${LSPMUX_STUB}" bash "${POST_FILE_EDIT}"
if [ "${RUN_RC}" -eq 0 ] && assert_stdout_empty && assert_stderr_contains "failed to parse hook input JSON"; then
    pass "post-file-edit.sh logs parse failures to stderr"
else
    fail "post-file-edit.sh did not handle malformed JSON as expected"
fi

run_hook '{"file_path": "src/main.py"}' env LSPMUX_PATH="${LSPMUX_STUB}" bash "${POST_FILE_EDIT}"
if [ "${RUN_RC}" -eq 0 ] && assert_stdout_empty && assert_stderr_empty; then
    pass "post-file-edit.sh stays silent for non-Rust files"
else
    fail "post-file-edit.sh emitted unexpected output for non-Rust files"
fi

run_hook '{"file_path": "src/main.rs"}' env LSPMUX_PATH="${LSPMUX_STUB}" bash "${POST_FILE_EDIT}"
if [ "${RUN_RC}" -eq 0 ] && assert_stdout_empty && assert_stderr_contains "sync failed:" && assert_stderr_contains "simulated sync failure"; then
    pass "post-file-edit.sh logs sync failures to stderr"
else
    fail "post-file-edit.sh did not surface sync failure details"
fi

# --- session-start.sh stays status-only ---
echo "-- SessionStart behavior --"
if ! grep -qE 'launchctl|systemctl|server &' "${SESSION_START}"; then
    pass "session-start.sh does not start services directly"
else
    fail "session-start.sh still contains bootstrap logic"
fi

run_hook '' env LSPMUX_PATH="${LSPMUX_STUB}" LSPMUX_BOOTSTRAP=off WORKSPACE_ROOT="/tmp/workspace" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 0 ] && assert_stdout_json && assert_stdout_contains "bootstrap disabled" && assert_stderr_empty; then
    pass "session-start.sh emits JSON for bootstrap=off"
else
    fail "session-start.sh did not emit the expected bootstrap=off payload"
fi

run_hook '' env LSPMUX_PATH="${LSPMUX_STUB}" LSPMUX_BOOTSTRAP=auto WORKSPACE_ROOT="/tmp/workspace" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 0 ] && assert_stdout_json && assert_stdout_contains "The MCP runtime will handle bootstrap on first use." && assert_stderr_empty; then
    pass "session-start.sh emits JSON guidance for bootstrap=auto"
else
    fail "session-start.sh did not emit the expected bootstrap=auto payload"
fi

run_hook '' env LSPMUX_PATH="${LSPMUX_STUB}" LSPMUX_BOOTSTRAP=require WORKSPACE_ROOT="/tmp/workspace" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 2 ] && assert_stdout_json && assert_stdout_contains "bootstrap is required" && assert_stderr_empty; then
    pass "session-start.sh emits warning JSON for bootstrap=require"
else
    fail "session-start.sh did not emit the expected bootstrap=require payload"
fi

run_hook '' env LSPMUX_PATH="${RUN_DIR}/missing-lspmux" LSPMUX_BOOTSTRAP=auto WORKSPACE_ROOT="/tmp/workspace" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 2 ] && assert_stdout_json && assert_stdout_contains "lspmux not installed" && assert_stderr_empty; then
    pass "session-start.sh emits warning JSON when lspmux is missing"
else
    fail "session-start.sh did not emit the expected missing-binary payload"
fi

# --- Summary ---
echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
