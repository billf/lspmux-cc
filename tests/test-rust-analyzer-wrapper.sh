#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WRAPPER="${SCRIPT_DIR}/plugins/lspmux-rust-cc/bin/rust-analyzer"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

HOME_DIR="${TMPDIR}/home"
PATH_DIR="${TMPDIR}/path-bin"
ENV_DIR="${TMPDIR}/env-bin"
mkdir -p "${HOME_DIR}" "${PATH_DIR}" "${ENV_DIR}"
BASE_PATH="/usr/bin:/bin:/usr/sbin:/sbin"

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

env_output="$(
    run_wrapper RUST_ANALYZER_PATH="${ENV_DIR}/rust-analyzer" "${WRAPPER}" --version
)"
if [ "${env_output}" != "env:--version" ]; then
    echo "expected explicit env binary to win, got ${env_output}" >&2
    exit 1
fi

path_output="$(run_wrapper "${WRAPPER}" --version)"
if [ "${path_output}" != "path:--version" ]; then
    echo "expected PATH rust-analyzer to be used, got ${path_output}" >&2
    exit 1
fi

mkdir -p "${SCRIPT_DIR}/result-rust-analyzer-nightly/bin"
trap 'rm -rf "${TMPDIR}" "${SCRIPT_DIR}/result-rust-analyzer-nightly"' EXIT
cat > "${SCRIPT_DIR}/result-rust-analyzer-nightly/bin/rust-analyzer" <<'EOF'
#!/usr/bin/env bash
printf 'legacy-result:%s\n' "$*"
EOF
chmod +x "${SCRIPT_DIR}/result-rust-analyzer-nightly/bin/rust-analyzer"

path_output_with_legacy_present="$(run_wrapper "${WRAPPER}" status)"
if [ "${path_output_with_legacy_present}" != "path:status" ]; then
    echo "expected wrapper to ignore legacy result symlink, got ${path_output_with_legacy_present}" >&2
    exit 1
fi

stderr_file="${TMPDIR}/wrapper.stderr"
if env -i HOME="${HOME_DIR}" PATH="${TMPDIR}/missing:${BASE_PATH}" "${WRAPPER}" >"${TMPDIR}/wrapper.stdout" 2>"${stderr_file}"; then
    echo "expected wrapper to fail without env or PATH binary" >&2
    exit 1
fi

if ! grep -q "Set RUST_ANALYZER_PATH or add rust-analyzer to PATH." "${stderr_file}"; then
    echo "expected missing-binary guidance in stderr" >&2
    exit 1
fi

echo "rust-analyzer wrapper lookup verified"
