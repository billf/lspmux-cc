manifest := "--manifest-path mcp-server/Cargo.toml"

# Run cargo check on mcp-server
check:
    cargo check {{manifest}} --all-targets

# Build mcp-server
build:
    cargo build {{manifest}} --all-targets

# Run clippy with pedantic/nursery warnings
clippy:
    cargo clippy {{manifest}} --all-targets -- -W clippy::nursery -W clippy::pedantic

# Format all Rust code
fmt:
    cargo fmt {{manifest}} --all

# Check formatting without modifying
fmt-check:
    cargo fmt {{manifest}} --all -- --check

# Run tests
test:
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
integration-test:
    cargo test {{manifest}} -- --ignored

# Lint all shell scripts
shellcheck:
    shellcheck setup bin/* plugins/**/hooks/scripts/*.sh plugins/**/bin/*

# Pre-push: check + clippy + fmt + test
pre-push: check clippy fmt-check test
