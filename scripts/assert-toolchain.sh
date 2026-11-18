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
# Nix governance is structural, not a trusted variable: IN_NIX_SHELL
# can be spoofed, inherited, or set by legacy nix-shell, so the Nix check applies
# only when the ambient rustc actually resolves into the Nix store. Its version
# must still match the pin so stale development shells fail closed.
# The cargo check compares release trains (major.minor), not exact versions: the
# rustc sysroot identity above is the exact gate; cargo only needs to be
# from the same train since the same rustc produces identical artifacts.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# Extract the major.minor train from `cargo --version` output on stdin.
cargo_train() {
    grep -oE '[0-9]+\.[0-9]+' | head -n 1 || true
}

PIN="$(bash "${SCRIPT_DIR}/scripts/toolchain-pin.sh" "${SCRIPT_DIR}/rust-toolchain.toml" 2>/dev/null || true)"
if [ -z "${PIN:-}" ]; then
    echo "ERROR: no channel found in rust-toolchain.toml" >&2
    exit 1
fi

nix_governed=false
if [ -n "${IN_NIX_SHELL:-}" ] && command -v rustc >/dev/null 2>&1; then
    nix_sysroot="$(rustc --print sysroot 2>/dev/null || true)"
    case "${nix_sysroot}" in
        /nix/store/*) nix_governed=true ;;
    esac
fi
if [ "${nix_governed}" = true ]; then
    nix_version="$(rustc --version | awk '{print $2}')"
    nix_cargo_train="$(cargo --version 2>/dev/null | cargo_train)"
    if [ "${nix_version}" != "${PIN}" ] || [ "${nix_cargo_train}" != "${PIN%.*}" ]; then
        echo "ERROR: stale nix develop toolchain (rustc ${nix_version}, cargo ${nix_cargo_train:-unknown}); expected pin '${PIN}'" >&2
        echo "Exit this shell and re-enter nix develop." >&2
        exit 1
    fi
    echo "toolchain guard: nix develop cargo + rustc match pin '${PIN}'"
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

PIN_CARGO_TRAIN="$(rustup run "${PIN}" cargo --version 2>/dev/null | cargo_train)"
AMBIENT_CARGO_TRAIN="$(cargo --version 2>/dev/null | cargo_train)"
if [ -z "${PIN_CARGO_TRAIN:-}" ] || [ -z "${AMBIENT_CARGO_TRAIN:-}" ] || [ "${PIN_CARGO_TRAIN}" != "${AMBIENT_CARGO_TRAIN}" ]; then
    echo "ERROR: ambient cargo (${AMBIENT_CARGO_TRAIN:-unknown}) disagrees with pinned toolchain '${PIN}' (cargo ${PIN_CARGO_TRAIN:-unknown})" >&2
    echo "Build with the pinned toolchain's cargo or enter nix develop." >&2
    exit 1
fi

echo "toolchain guard: ambient cargo ${AMBIENT_CARGO_TRAIN} + rustc sysroot match pin '${PIN}'"
