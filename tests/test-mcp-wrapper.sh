#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WRAPPER="${SCRIPT_DIR}/plugins/lspmux-rust-cc/bin/lspmux-cc-mcp"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

HOME_DIR="${TMPDIR}/home"
mkdir -p "${HOME_DIR}/.cargo/bin"

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

default_output="$(run_wrapper "${WRAPPER}")"
IFS='|' read -r default_kind default_host default_session <<<"${default_output}"

if [ "${default_kind}" != "claude_mcp" ]; then
    echo "expected default kind claude_mcp, got ${default_kind}" >&2
    exit 1
fi

if [ "${default_host}" != "claude" ]; then
    echo "expected default host claude, got ${default_host}" >&2
    exit 1
fi

if ! [[ "${default_session}" =~ ^claude-mcp-[0-9]+-[0-9]+$ ]]; then
    echo "expected default session id with claude-mcp prefix, got ${default_session}" >&2
    exit 1
fi

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

if [ "${custom_kind}" != "custom_kind" ]; then
    echo "expected caller-provided kind to be preserved, got ${custom_kind}" >&2
    exit 1
fi

if [ "${custom_host}" != "custom_host" ]; then
    echo "expected caller-provided host to be preserved, got ${custom_host}" >&2
    exit 1
fi

if [ "${custom_session}" != "custom-session" ]; then
    echo "expected caller-provided session to be preserved, got ${custom_session}" >&2
    exit 1
fi

echo "wrapper env defaults verified"
