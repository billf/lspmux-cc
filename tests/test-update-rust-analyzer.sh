#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
UPDATER="${SCRIPT_DIR}/bin/update-rust-analyzer"

TMPROOT="$(mktemp -d)"
trap 'rm -rf "${TMPROOT}"' EXIT

pass() { echo "  PASS: $*"; }
fail() { echo "  FAIL: $*" >&2; exit 1; }

make_shims() {
    local shim_dir="$1"
    mkdir -p "${shim_dir}"

    cat >"${shim_dir}/uname" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "-m" ]; then
    printf '%s\n' "${TEST_UNAME_M:-arm64}"
    exit 0
fi

exec /usr/bin/uname "$@"
EOF

    cat >"${shim_dir}/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

output=""
url=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o)
            output="$2"
            shift 2
            ;;
        --proto|--max-redirs)
            shift 2
            ;;
        -L|--fail|--silent|--show-error)
            shift
            ;;
        *)
            url="$1"
            shift
            ;;
    esac
done

case "${url}" in
    "${TEST_LATEST_URL}")
        cat "${TEST_RELEASE_JSON}"
        ;;
    "${TEST_DOWNLOAD_URL}")
        [ -n "${output}" ] || {
            echo "curl shim missing -o for download" >&2
            exit 1
        }
        cp "${TEST_DOWNLOAD_PAYLOAD}" "${output}"
        ;;
    "${TEST_CHECKSUMS_URL}")
        [ -n "${output}" ] || {
            echo "curl shim missing -o for checksum asset" >&2
            exit 1
        }
        cp "${TEST_CHECKSUMS_FILE}" "${output}"
        ;;
    *)
        echo "unexpected curl URL: ${url}" >&2
        exit 1
        ;;
esac
EOF

    cat >"${shim_dir}/gunzip" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

file=""
for arg in "$@"; do
    case "$arg" in
        -f)
            ;;
        *)
            file="$arg"
            ;;
    esac
done

[ -n "${file}" ] || {
    echo "gunzip shim missing input" >&2
    exit 1
}

mv "${file}" "${file%.gz}"
EOF

    cat >"${shim_dir}/file" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf '%s: Mach-O 64-bit executable arm64\n' "${1:-/dev/null}"
EOF

    cat >"${shim_dir}/shasum" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "-a" ] && [ "${2:-}" = "256" ] && [ -n "${3:-}" ]; then
    printf '%s  %s\n' "${TEST_SHA256:?missing TEST_SHA256}" "$3"
    exit 0
fi

echo "unexpected shasum invocation" >&2
exit 1
EOF

    chmod +x "${shim_dir}/uname" "${shim_dir}/curl" "${shim_dir}/gunzip" "${shim_dir}/file" "${shim_dir}/shasum"
}

write_release_fixture() {
    local release_json="$1"
    local download_url="$2"
    local checksum_url="$3"
    local tag="$4"

    cat >"${release_json}" <<EOF
{
  "tag_name": "${tag}",
  "assets": [
    {
      "name": "rust-analyzer-aarch64-apple-darwin.gz",
      "browser_download_url": "${download_url}"
    },
    {
      "name": "SHA256SUMS",
      "browser_download_url": "${checksum_url}"
    }
  ]
}
EOF
}

scenario_success() {
    local root="${TMPROOT}/success"
    mkdir -p "${root}"
    local shim_dir="${root}/shims"
    make_shims "${shim_dir}"

    local release_json="${root}/release.json"
    local payload="${root}/download.bin"
    local checksums="${root}/SHA256SUMS"
    local home="${root}/home"
    local data_home="${root}/data"
    mkdir -p "${home}" "${data_home}"

    printf 'fake binary payload\n' >"${payload}"
    printf 'abc1234567890deadbeef  rust-analyzer-aarch64-apple-darwin.gz\n' >"${checksums}"
    write_release_fixture "${release_json}" "https://example.test/download.gz" "https://example.test/SHA256SUMS" "v1.2.3"

    local stdout_file="${root}/stdout"
    local stderr_file="${root}/stderr"
    set +e
    HOME="${home}" \
    XDG_DATA_HOME="${data_home}" \
    PATH="${shim_dir}:${PATH}" \
    TEST_UNAME_M=arm64 \
    TEST_SHA256=abc1234567890deadbeef \
    TEST_RELEASE_JSON="${release_json}" \
    TEST_DOWNLOAD_PAYLOAD="${payload}" \
    TEST_CHECKSUMS_FILE="${checksums}" \
    TEST_LATEST_URL=https://api.github.com/repos/rust-lang/rust-analyzer/releases/latest \
    TEST_DOWNLOAD_URL=https://example.test/download.gz \
    TEST_CHECKSUMS_URL=https://example.test/SHA256SUMS \
    bash "${UPDATER}" >"${stdout_file}" 2>"${stderr_file}"
    local status=$?
    set -e
    [ "${status}" -eq 0 ] || {
        echo "FAIL: success returned ${status}, expected 0" >&2
        cat "${stdout_file}" >&2 || true
        cat "${stderr_file}" >&2 || true
        exit 1
    }

    local ra_binary="${data_home}/lspmux-rust-analyzer/bin/v1.2.3/rust-analyzer"
    local current_link="${data_home}/lspmux-rust-analyzer/current"

    [ -x "${ra_binary}" ] || fail "expected installed binary at ${ra_binary}"
    [ -L "${current_link}" ] || fail "expected current symlink at ${current_link}"
    [ "$(readlink "${current_link}")" = "${data_home}/lspmux-rust-analyzer/bin/v1.2.3" ] || \
        fail "current symlink pointed to unexpected location"

    pass "success scenario installed and linked rust-analyzer"
}

scenario_checksum_mismatch() {
    local root="${TMPROOT}/mismatch"
    mkdir -p "${root}"
    local shim_dir="${root}/shims"
    make_shims "${shim_dir}"

    local release_json="${root}/release.json"
    local payload="${root}/download.bin"
    local checksums="${root}/SHA256SUMS"
    local home="${root}/home"
    local data_home="${root}/data"
    mkdir -p "${home}" "${data_home}"

    printf 'fake binary payload\n' >"${payload}"
    printf 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff  rust-analyzer-aarch64-apple-darwin.gz\n' >"${checksums}"
    write_release_fixture "${release_json}" "https://example.test/download.gz" "https://example.test/SHA256SUMS" "v1.2.4"

    local stdout_file="${root}/stdout"
    local stderr_file="${root}/stderr"
    set +e
    HOME="${home}" \
    XDG_DATA_HOME="${data_home}" \
    PATH="${shim_dir}:${PATH}" \
    TEST_UNAME_M=arm64 \
    TEST_SHA256=abc1234567890deadbeef \
    TEST_RELEASE_JSON="${release_json}" \
    TEST_DOWNLOAD_PAYLOAD="${payload}" \
    TEST_CHECKSUMS_FILE="${checksums}" \
    TEST_LATEST_URL=https://api.github.com/repos/rust-lang/rust-analyzer/releases/latest \
    TEST_DOWNLOAD_URL=https://example.test/download.gz \
    TEST_CHECKSUMS_URL=https://example.test/SHA256SUMS \
    bash "${UPDATER}" >"${stdout_file}" 2>"${stderr_file}"
    local status=$?
    set -e
    [ "${status}" -eq 1 ] || {
        echo "FAIL: checksum mismatch returned ${status}, expected 1" >&2
        cat "${stdout_file}" >&2 || true
        cat "${stderr_file}" >&2 || true
        exit 1
    }

    local ra_dir="${data_home}/lspmux-rust-analyzer"
    [ ! -e "${ra_dir}/bin/v1.2.4/rust-analyzer" ] || fail "binary should not be installed on checksum mismatch"
    [ ! -e "${ra_dir}/current" ] || fail "current symlink should not be updated on checksum mismatch"
    grep -q "checksum mismatch" "${stderr_file}" || fail "stderr should mention checksum mismatch"

    pass "checksum mismatch scenario failed closed"
}

scenario_missing_checksum_asset() {
    local root="${TMPROOT}/missing-checksum"
    mkdir -p "${root}"
    local shim_dir="${root}/shims"
    make_shims "${shim_dir}"

    local release_json="${root}/release.json"
    local payload="${root}/download.bin"
    local home="${root}/home"
    local data_home="${root}/data"
    mkdir -p "${home}" "${data_home}"

    printf 'fake binary payload\n' >"${payload}"
    cat >"${release_json}" <<EOF
{
  "tag_name": "v1.2.5",
  "assets": [
    {
      "name": "rust-analyzer-aarch64-apple-darwin.gz",
      "browser_download_url": "https://example.test/download.gz"
    }
  ]
}
EOF

    local stdout_file="${root}/stdout"
    local stderr_file="${root}/stderr"
    set +e
    HOME="${home}" \
    XDG_DATA_HOME="${data_home}" \
    PATH="${shim_dir}:${PATH}" \
    TEST_UNAME_M=arm64 \
    TEST_SHA256=abc1234567890deadbeef \
    TEST_RELEASE_JSON="${release_json}" \
    TEST_DOWNLOAD_PAYLOAD="${payload}" \
    TEST_LATEST_URL=https://api.github.com/repos/rust-lang/rust-analyzer/releases/latest \
    TEST_DOWNLOAD_URL=https://example.test/download.gz \
    bash "${UPDATER}" >"${stdout_file}" 2>"${stderr_file}"
    local status=$?
    set -e
    [ "${status}" -eq 1 ] || {
        echo "FAIL: missing checksum asset returned ${status}, expected 1" >&2
        cat "${stdout_file}" >&2 || true
        cat "${stderr_file}" >&2 || true
        exit 1
    }

    [ ! -e "${data_home}/lspmux-rust-analyzer" ] || fail "installer should not create version tree without checksum asset"
    grep -q "no checksum asset found" "${stderr_file}" || fail "stderr should mention missing checksum asset"

    pass "missing checksum asset scenario failed closed"
}

scenario_missing_download_asset() {
    local root="${TMPROOT}/missing-download"
    mkdir -p "${root}"
    local shim_dir="${root}/shims"
    make_shims "${shim_dir}"

    local release_json="${root}/release.json"
    local checksums="${root}/SHA256SUMS"
    local home="${root}/home"
    local data_home="${root}/data"
    mkdir -p "${home}" "${data_home}"

    printf 'abc1234567890deadbeef  rust-analyzer-aarch64-apple-darwin.gz\n' >"${checksums}"
    cat >"${release_json}" <<EOF
{
  "tag_name": "v1.2.6",
  "assets": [
    {
      "name": "SHA256SUMS",
      "browser_download_url": "https://example.test/SHA256SUMS"
    }
  ]
}
EOF

    local stdout_file="${root}/stdout"
    local stderr_file="${root}/stderr"
    set +e
    HOME="${home}" \
    XDG_DATA_HOME="${data_home}" \
    PATH="${shim_dir}:${PATH}" \
    TEST_UNAME_M=arm64 \
    TEST_SHA256=abc1234567890deadbeef \
    TEST_RELEASE_JSON="${release_json}" \
    TEST_CHECKSUMS_FILE="${checksums}" \
    TEST_LATEST_URL=https://api.github.com/repos/rust-lang/rust-analyzer/releases/latest \
    TEST_CHECKSUMS_URL=https://example.test/SHA256SUMS \
    bash "${UPDATER}" >"${stdout_file}" 2>"${stderr_file}"
    local status=$?
    set -e
    [ "${status}" -eq 1 ] || {
        echo "FAIL: missing download asset returned ${status}, expected 1" >&2
        cat "${stdout_file}" >&2 || true
        cat "${stderr_file}" >&2 || true
        exit 1
    }

    [ ! -e "${data_home}/lspmux-rust-analyzer" ] || fail "installer should not create version tree without download asset"
    grep -q "no download found" "${stderr_file}" || fail "stderr should mention missing download asset"

    pass "missing download asset scenario failed closed"
}

scenario_success
scenario_checksum_mismatch
scenario_missing_checksum_asset
scenario_missing_download_asset
