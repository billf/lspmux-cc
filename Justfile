manifest := "--manifest-path mcp-server/Cargo.toml"

# Assert ambient cargo matches rust-toolchain.toml (also detects stale nix develop shells)
toolchain:
    ./scripts/assert-toolchain.sh

# Run cargo check on mcp-server
check: toolchain
    cargo check {{manifest}} --all-targets

# Build mcp-server
build: toolchain
    cargo build {{manifest}} --all-targets

# Run clippy with pedantic/nursery warnings
clippy: toolchain
    cargo clippy {{manifest}} --all-targets -- -W clippy::nursery -W clippy::pedantic

# Format all Rust code
fmt:
    cargo fmt {{manifest}} --all

# Check formatting without modifying
fmt-check:
    cargo fmt {{manifest}} --all -- --check

# Run tests
test: toolchain
    cargo test {{manifest}}

# Run nix flake check (clippy + fmt + tests)
nix-check:
    nix flake check

# Build via nix
nix-build:
    nix build

# Run the setup script
setup:
    ./setup

# Run integration tests (requires lspmux + rust-analyzer binaries)
integration-test: toolchain
    cargo test {{manifest}} -- --ignored

# Lint all shell scripts
shellcheck:
    shellcheck setup bin/* hooks/scripts/*.sh scripts/*.sh tests/*.sh

# Build the assembled plugin via nix
nix-plugin:
    nix build .#plugin -o result-plugin

# Pre-push: check + clippy + fmt + test
pre-push: check clippy fmt-check test
