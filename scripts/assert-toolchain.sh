#!/usr/bin/env bash
set -euo pipefail

# Assert the ambient cargo/rustc resolve to the rust-toolchain.toml pin.
#
# rust-analyzer builds workspace metadata with the cargo from its own
# environment, so an ambient toolchain that disagrees with the pin means
# target/ artifacts (and proc-macro dylib versions) churn between the
# developer's builds and rust-analyzer's. Funnel everything through one
# toolchain: this pin outside Nix, the flake's locked toolchain inside it.
#
# Skipped inside `nix develop` (IN_NIX_SHELL is set): the flake governs the
# toolchain there and rust-toolchain.toml is inert for Nix-provided cargo.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

PIN="$(grep -E '^[[:space:]]*channel[[:space:]]*=' "${SCRIPT_DIR}/rust-toolchain.toml" 2>/dev/null | head -n 1 | sed -E 's/^[^"]*"([^"]+)".*/\1/')"
if [ -z "${PIN:-}" ]; then
    echo "ERROR: no channel found in rust-toolchain.toml" >&2
    exit 1
fi

if [ -n "${IN_NIX_SHELL:-}" ]; then
    echo "toolchain guard: in nix develop, skipping pin check (flake governs)"
    exit 0
fi

if ! command -v rustup >/dev/null 2>&1; then
    echo "ERROR: rustup not found on PATH; install it or enter nix develop" >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
    echo "ERROR: cargo/rustc not found on PATH" >&2
    exit 1
fi

PIN_SYSROOT="$(rustup run "${PIN}" rustc --print sysroot 2>/dev/null)" || {
    echo "ERROR: pinned toolchain '${PIN}' is not installed" >&2
    echo "Run: rustup toolchain install ${PIN}" >&2
    exit 1
}

AMBIENT_SYSROOT="$(rustc --print sysroot 2>/dev/null)" || {
    echo "ERROR: rustc --print sysroot failed" >&2
    exit 1
}

if [ "${PIN_SYSROOT}" != "${AMBIENT_SYSROOT}" ]; then
    echo "ERROR: ambient toolchain disagrees with rust-toolchain.toml pin '${PIN}'" >&2
    echo "  ambient: ${AMBIENT_SYSROOT}" >&2
    echo "  pinned:  ${PIN_SYSROOT}" >&2
    echo "Put rustup shims first on PATH (so the pin applies) or enter nix develop." >&2
    exit 1
fi

echo "toolchain guard: ambient cargo matches pin '${PIN}'"
