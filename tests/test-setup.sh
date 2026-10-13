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
# SC2016 off for the grep patterns: single quotes are intentional, the
# patterns must match the literal text (including "${...}") in the sources.
# shellcheck disable=SC2016
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

# --- toolchain guard matrix ---
# Executable coverage for scripts/assert-toolchain.sh via stub toolchains on
# a controlled PATH. Each case pins the stub outputs, never the real tools.
echo "-- toolchain guard matrix --"
GUARD_DIR="$(mktemp -d)"
trap 'rm -rf "${GUARD_DIR}" "${LSPMUX_VALIDATOR_TMP}"' EXIT
mkdir -p "${GUARD_DIR}/bin" "${GUARD_DIR}/proj/scripts"
cp "${SCRIPT_DIR}/scripts/"{assert-toolchain,toolchain-pin}.sh "${GUARD_DIR}/proj/scripts/"
printf '[toolchain]\nchannel = "1.97.1"\n' > "${GUARD_DIR}/proj/rust-toolchain.toml"

cat > "${GUARD_DIR}/bin/rustup" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = "run" ] && [ "$2" = "1.97.1" ] && [ "$3" = "rustc" ]; then printf '%s\n' "$FAKE_PIN_SYSROOT"; exit 0; fi
if [ "$1" = "run" ] && [ "$2" = "1.97.1" ] && [ "$3" = "cargo" ]; then printf 'cargo 1.97.1 (a1b2c3d 2026-01-01)\n'; exit 0; fi
exit 1
EOF
cat > "${GUARD_DIR}/bin/rustc" <<'EOF'
#!/usr/bin/env bash
[ "$1 $2" = "--print sysroot" ] && printf '%s\n' "$FAKE_AMB_SYSROOT" && exit 0
printf 'rustc %s (x)\n' "${FAKE_RUSTC_VERSION:-1.97.1}"; exit 0
EOF
cat > "${GUARD_DIR}/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$FAKE_AMB_CARGO_VERSION"; exit 0
EOF
chmod +x "${GUARD_DIR}/bin/"*

# run_guard <expected-exit> <expected-message> <VAR=val>...
run_guard() {
    local expected="$1"
    local message="$2"
    shift 2
    local got
    got=0
    env -i "$@" PATH="${GUARD_DIR}/bin:/usr/bin:/bin" HOME="${HOME}" \
        "${GUARD_DIR}/proj/scripts/assert-toolchain.sh" >"${GUARD_DIR}/output" 2>&1 || got="$?"
    if [ "${got}" = "${expected}" ] && grep -Fq "${message}" "${GUARD_DIR}/output"; then
        pass "guard exits ${expected} ($*)"
    else
        fail "guard exits ${got}, expected ${expected} with message ${message} ($*)"
    fi
}

run_guard 0 "match pin" \
    FAKE_PIN_SYSROOT=/rustup/pin FAKE_AMB_SYSROOT=/rustup/pin \
    FAKE_AMB_CARGO_VERSION="cargo 1.97.0 (y)"
run_guard 1 "ambient toolchain disagrees" \
    FAKE_PIN_SYSROOT=/rustup/pin FAKE_AMB_SYSROOT=/other \
    FAKE_AMB_CARGO_VERSION="cargo 1.97.0 (y)"
run_guard 0 "match pin" \
    FAKE_PIN_SYSROOT=/rustup/pin FAKE_AMB_SYSROOT=/nix/store/abc-rustc \
    FAKE_AMB_CARGO_VERSION="cargo 1.97.0 (y)" IN_NIX_SHELL=impure
run_guard 1 "ambient toolchain disagrees" \
    FAKE_PIN_SYSROOT=/rustup/pin FAKE_AMB_SYSROOT=/other \
    FAKE_AMB_CARGO_VERSION="cargo 1.97.0 (y)" IN_NIX_SHELL=bogus
run_guard 1 "ambient cargo" \
    FAKE_PIN_SYSROOT=/s FAKE_AMB_SYSROOT=/s \
    FAKE_AMB_CARGO_VERSION="cargo 1.99.0 (z)"

run_guard 1 "stale nix develop" \
    FAKE_PIN_SYSROOT=/rustup/pin FAKE_AMB_SYSROOT=/nix/store/old-rustc \
    FAKE_RUSTC_VERSION=1.96.0 FAKE_AMB_CARGO_VERSION="cargo 1.96.0 (y)" IN_NIX_SHELL=impure

# Missing rustup: strip the stub dir from PATH.
if env -i PATH="/usr/bin:/bin" HOME="${HOME}" \
    "${GUARD_DIR}/proj/scripts/assert-toolchain.sh" >"${GUARD_DIR}/output" 2>&1; then
    fail "guard passes without rustup"
elif grep -Fq "rustup not found" "${GUARD_DIR}/output"; then
    pass "guard fails without rustup"
else
    fail "missing rustup hides the cause"
fi

# rustup alone is insufficient: name the missing cargo/rustc cause.
mkdir -p "${GUARD_DIR}/bin-missing"
ln -s "${GUARD_DIR}/bin/rustup" "${GUARD_DIR}/bin-missing/rustup"
if env -i PATH="${GUARD_DIR}/bin-missing:/usr/bin:/bin" HOME="${HOME}" \
    "${GUARD_DIR}/proj/scripts/assert-toolchain.sh" >"${GUARD_DIR}/output" 2>&1; then
    fail "guard passes without cargo/rustc"
elif grep -Fq "cargo/rustc not found" "${GUARD_DIR}/output"; then
    pass "missing cargo/rustc reports the cause"
else
    fail "missing cargo/rustc hides the cause"
fi

# Pin toolchain not installed: rustup present but `run` fails.
mkdir -p "${GUARD_DIR}/bin-nopin"
cat > "${GUARD_DIR}/bin-nopin/rustup" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
ln -s "${GUARD_DIR}/bin/rustc" "${GUARD_DIR}/bin-nopin/rustc"
ln -s "${GUARD_DIR}/bin/cargo" "${GUARD_DIR}/bin-nopin/cargo"
chmod +x "${GUARD_DIR}/bin-nopin/rustup"
if env -i PATH="${GUARD_DIR}/bin-nopin:/usr/bin:/bin" HOME="${HOME}" \
    FAKE_AMB_SYSROOT=/s FAKE_AMB_CARGO_VERSION="cargo 1.97.0 (y)" \
    "${GUARD_DIR}/proj/scripts/assert-toolchain.sh" 2>"${GUARD_DIR}/stderr" >/dev/null; then
    fail "guard passes with pin toolchain missing"
elif grep -q "not installed" "${GUARD_DIR}/stderr"; then
    pass "missing pin toolchain reports not-installed"
else
    fail "missing pin toolchain hides the cause"
fi

# Channel-less pin file: friendly error names the rule.
printf '[toolchain]\n# no channel here\n' > "${GUARD_DIR}/proj/rust-toolchain.toml"
if env -i PATH="${GUARD_DIR}/bin:/usr/bin:/bin" HOME="${HOME}" \
    FAKE_PIN_SYSROOT=/s FAKE_AMB_SYSROOT=/s FAKE_AMB_CARGO_VERSION="cargo 1.97.0 (y)" \
    "${GUARD_DIR}/proj/scripts/assert-toolchain.sh" 2>"${GUARD_DIR}/stderr" >/dev/null; then
    fail "guard passes with channel-less pin"
elif grep -q "no channel found" "${GUARD_DIR}/stderr"; then
    pass "channel-less pin reports no-channel-found"
else
    fail "channel-less pin hides the cause"
fi

# Single-quoted TOML spelling parses to the same pin.
printf "[toolchain]\nchannel = '1.97.1'\n" > "${GUARD_DIR}/proj/rust-toolchain.toml"
if env -i PATH="${GUARD_DIR}/bin:/usr/bin:/bin" HOME="${HOME}" \
    FAKE_PIN_SYSROOT=/rustup/pin FAKE_AMB_SYSROOT=/rustup/pin \
    FAKE_AMB_CARGO_VERSION="cargo 1.97.0 (y)" \
    "${GUARD_DIR}/proj/scripts/assert-toolchain.sh" >/dev/null 2>&1; then
    pass "single-quoted channel parses like double-quoted"
else
    fail "single-quoted channel rejected"
fi

# Both gates use this exact parser; exercise both accepted quote spellings
# and reject bare TOML values rather than returning the entire channel line.
for channel in '"1.97.1"' "'1.97.1'"; do
    printf '[toolchain]\nchannel = %s # comment\n' "${channel}" > "${GUARD_DIR}/proj/rust-toolchain.toml"
    if [ "$(bash "${SCRIPT_DIR}/scripts/toolchain-pin.sh" "${GUARD_DIR}/proj/rust-toolchain.toml")" = 1.97.1 ]; then
        pass "shared flake/guard parser accepts ${channel}"
    else
        fail "shared flake/guard parser rejects ${channel}"
    fi
done
printf '[toolchain]\nchannel = 1.97.1\n' > "${GUARD_DIR}/proj/rust-toolchain.toml"
run_guard 1 "no channel found"
# Literal Nix interpolation, not shell variables.
# shellcheck disable=SC2016
if grep -Fq 'bash ${./scripts/toolchain-pin.sh} ${./rust-toolchain.toml}' "${SCRIPT_DIR}/flake.nix"; then
    pass "flake uses the guard pin parser"
else
    fail "flake parser diverges from guard"
fi

# --- shell gate coverage ---
# Every tracked shell file must be covered by the shellcheck gates, and both
# gates (CI + Justfile) must enumerate the same set — otherwise a new script
# ships unlinted with all gates green.
echo "-- shell gate coverage --"
gate_miss=0
for pattern in setup 'bin/*' 'hooks/scripts/*.sh' 'scripts/*.sh' 'tests/*.sh'; do
    if ! grep -Fq "${pattern}" "${SCRIPT_DIR}/.github/workflows/ci.yml" \
        || ! grep -Fq "${pattern}" "${SCRIPT_DIR}/Justfile"; then
        echo "  gate enumeration missing pattern: ${pattern}"
        gate_miss=1
    fi
done
while IFS= read -r shell_file; do
    case "${shell_file}" in
        setup|bin/*|hooks/scripts/*.sh|scripts/*.sh|tests/*.sh) ;;
        *)
            echo "  shell file outside gate coverage: ${shell_file}"
            gate_miss=1
            ;;
    esac
done < <(git -C "${SCRIPT_DIR}" ls-files '*.sh' 'bin/*')
if [ "${gate_miss}" = 0 ]; then
    pass "all tracked shell files match the shellcheck gates"
else
    fail "shell gate coverage gap (see lines above)"
fi

# --- Summary ---
echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ] || exit 1
