#!/usr/bin/env bash
set -euo pipefail

# Tests M1 behavior: session-start.sh reports workspace match/mismatch honestly
# based on `lspmux status --json` output.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PLUGIN_DIR="${SCRIPT_DIR}"
SESSION_START="${PLUGIN_DIR}/hooks/scripts/session-start.sh"
RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/lspmux-m1.XXXXXX")"
PASS=0
FAIL=0
RUN_RC=0
RUN_STDOUT_FILE=""
RUN_STDERR_FILE=""

trap 'rm -rf "${RUN_DIR}"' EXIT

pass() { echo "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $*" >&2; FAIL=$((FAIL + 1)); }

run_hook() {
    RUN_STDOUT_FILE="$(mktemp "${RUN_DIR}/stdout.XXXXXX")"
    RUN_STDERR_FILE="$(mktemp "${RUN_DIR}/stderr.XXXXXX")"

    set +e
    printf '' | "$@" >"${RUN_STDOUT_FILE}" 2>"${RUN_STDERR_FILE}"
    RUN_RC=$?
    set -e
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

assert_stdout_not_contains() {
    local needle="$1"
    if ! grep -Fq "${needle}" "${RUN_STDOUT_FILE}"; then
        return 0
    fi
    fail "stdout unexpectedly contained: ${needle}"
    sed -n '1,20p' "${RUN_STDOUT_FILE}" >&2
    return 1
}

# Build a stub lspmux that returns a fixed `status --json` payload describing
# two instances at the two workspace paths passed in.
make_lspmux_stub() {
    local ws_a="$1"
    local ws_b="$2"
    local stub="${RUN_DIR}/lspmux-stub"
    cat >"${stub}" <<EOF
#!/usr/bin/env bash
case "\${1:-}" in
    status)
        if [ "\${2:-}" = "--json" ]; then
            cat <<'JSON'
{
    "instances": [
        {
            "pid": 11111,
            "workspaceRoot": { "path": "${ws_a}" },
            "idleFor": 500,
            "clients": []
        },
        {
            "pid": 22222,
            "workspaceRoot": { "path": "${ws_b}" },
            "idleFor": 999,
            "clients": []
        }
    ]
}
JSON
            exit 0
        fi
        exit 0
        ;;
    *)
        exit 0
        ;;
esac
EOF
    chmod +x "${stub}"
    printf '%s\n' "${stub}"
}

make_dead_lspmux_stub() {
    local stub="${RUN_DIR}/lspmux-dead"
    cat >"${stub}" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "${stub}"
    printf '%s\n' "${stub}"
}

echo "=== M1 SessionStart workspace-mismatch tests ==="

WS_A="$(mktemp -d "${RUN_DIR}/wt-a.XXXXXX")"
WS_B="$(mktemp -d "${RUN_DIR}/wt-b.XXXXXX")"
WS_A_CANON="$(cd "${WS_A}" && pwd -P)"
WS_B_CANON="$(cd "${WS_B}" && pwd -P)"
LSPMUX_STUB="$(make_lspmux_stub "${WS_A_CANON}" "${WS_B_CANON}")"

# --- Match: requested workspace appears in served instances ---
echo "-- Match path --"
run_hook env LSPMUX_PATH="${LSPMUX_STUB}" LSPMUX_BOOTSTRAP=auto \
    WORKSPACE_ROOT="${WS_A_CANON}" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 0 ] \
    && assert_stdout_contains "serving this workspace" \
    && assert_stdout_contains "11111" \
    && assert_stdout_not_contains "WARNING"; then
    pass "match reports pid + idle and no WARNING"
else
    fail "match path did not produce expected message"
fi

# --- Mismatch: requested workspace is not in served instances ---
echo "-- Mismatch path --"
WS_OTHER="$(mktemp -d "${RUN_DIR}/wt-other.XXXXXX")"
WS_OTHER_CANON="$(cd "${WS_OTHER}" && pwd -P)"
run_hook env LSPMUX_PATH="${LSPMUX_STUB}" LSPMUX_BOOTSTRAP=auto \
    WORKSPACE_ROOT="${WS_OTHER_CANON}" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 0 ] \
    && assert_stdout_contains "WARNING" \
    && assert_stdout_contains "NOT yet serving" \
    && assert_stdout_contains "${WS_A_CANON}" \
    && assert_stdout_contains "${WS_B_CANON}"; then
    pass "mismatch surfaces WARNING and lists served workspaces"
else
    fail "mismatch path did not produce expected WARNING"
fi

# --- Daemon down: existing not-running-yet message still emitted ---
echo "-- Daemon down --"
DEAD_STUB="$(make_dead_lspmux_stub)"
run_hook env LSPMUX_PATH="${DEAD_STUB}" LSPMUX_BOOTSTRAP=auto \
    WORKSPACE_ROOT="${WS_A_CANON}" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 0 ] \
    && assert_stdout_contains "not running yet"; then
    pass "daemon-down emits not-running-yet guidance"
else
    fail "daemon-down path did not produce expected message"
fi

# --- Bootstrap=off: short-circuits before probing the daemon ---
echo "-- Bootstrap=off short-circuit --"
run_hook env LSPMUX_PATH="${LSPMUX_STUB}" LSPMUX_BOOTSTRAP=off \
    WORKSPACE_ROOT="${WS_OTHER_CANON}" bash "${SESSION_START}"
if [ "${RUN_RC}" -eq 0 ] \
    && assert_stdout_contains "bootstrap disabled" \
    && assert_stdout_not_contains "WARNING"; then
    pass "bootstrap=off skips daemon probe"
else
    fail "bootstrap=off did not short-circuit"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
