#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WRAPPER="${SCRIPT_DIR}/plugins/lspmux-rust-cc/bin/rust-analyzer"

TEST_DIR="$(mktemp -d)"
trap 'rm -rf "${TEST_DIR}" "${SCRIPT_DIR}/result-rust-analyzer-nightly"' EXIT

HOME_DIR="${TEST_DIR}/home"
PATH_DIR="${TEST_DIR}/path-bin"
ENV_DIR="${TEST_DIR}/env-bin"
mkdir -p "${HOME_DIR}" "${PATH_DIR}" "${ENV_DIR}"
BASE_PATH="/usr/bin:/bin:/usr/sbin:/sbin"

PASS=0
FAIL=0
pass() { echo "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $*" >&2; FAIL=$((FAIL + 1)); }

cat > "${PATH_DIR}/rust-analyzer" <<'EOF'
#!/usr/bin/env bash
printf 'path:%s\n' "$*"
EOF
chmod +x "${PATH_DIR}/rust-analyzer"

cat > "${ENV_DIR}/rust-analyzer" <<'EOF'
#!/usr/bin/env bash
printf 'env:%s\n' "$*"
EOF
chmod +x "${ENV_DIR}/rust-analyzer"

run_wrapper() {
    env -i HOME="${HOME_DIR}" PATH="${PATH_DIR}:${BASE_PATH}" "$@"
}

echo "=== Rust Analyzer Wrapper Tests ==="

echo "-- Binary resolution --"
env_output="$(
    run_wrapper RUST_ANALYZER_PATH="${ENV_DIR}/rust-analyzer" "${WRAPPER}" --version
)"
if [ "${env_output}" = "env:--version" ]; then
    pass "explicit env binary wins over PATH"
else
    fail "expected explicit env binary to win, got ${env_output}"
fi

path_output="$(run_wrapper "${WRAPPER}" --version)"
if [ "${path_output}" = "path:--version" ]; then
    pass "PATH rust-analyzer is used as fallback"
else
    fail "expected PATH rust-analyzer to be used, got ${path_output}"
fi

echo "-- Legacy result symlink --"
mkdir -p "${SCRIPT_DIR}/result-rust-analyzer-nightly/bin"
cat > "${SCRIPT_DIR}/result-rust-analyzer-nightly/bin/rust-analyzer" <<'EOF'
#!/usr/bin/env bash
printf 'legacy-result:%s\n' "$*"
EOF
chmod +x "${SCRIPT_DIR}/result-rust-analyzer-nightly/bin/rust-analyzer"

path_output_with_legacy_present="$(run_wrapper "${WRAPPER}" status)"
if [ "${path_output_with_legacy_present}" = "path:status" ]; then
    pass "wrapper ignores legacy result symlink"
else
    fail "expected wrapper to ignore legacy result symlink, got ${path_output_with_legacy_present}"
fi

echo "-- Missing binary error --"
stderr_file="${TEST_DIR}/wrapper.stderr"
if env -i HOME="${HOME_DIR}" PATH="${TEST_DIR}/missing:${BASE_PATH}" "${WRAPPER}" >"${TEST_DIR}/wrapper.stdout" 2>"${stderr_file}"; then
    fail "wrapper should fail without env or PATH binary"
else
    pass "wrapper exits non-zero without env or PATH binary"
fi

if grep -q "Set RUST_ANALYZER_PATH or add rust-analyzer to PATH." "${stderr_file}"; then
    pass "missing-binary guidance appears in stderr"
else
    fail "expected missing-binary guidance in stderr"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
