# lspmux-cc

LSP multiplexing for Claude Code via [lspmux-rust-analyzer](https://github.com/sunshowers/lspmux-rust-analyzer).

## Project Layout

- Shell-script-first project with a Rust sub-project under `mcp-server/`
- Nix flake provides devShell, packages, and checks
- Cargo workspace lives only under `mcp-server/`

## Build Commands

### Rust (mcp-server)

```bash
cargo check --manifest-path mcp-server/Cargo.toml
cargo build --manifest-path mcp-server/Cargo.toml
cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic
cargo fmt --manifest-path mcp-server/Cargo.toml --all
cargo test --manifest-path mcp-server/Cargo.toml
```

### Nix

```bash
nix flake check    # clippy + fmt + tests
nix build          # build mcp-server binary
nix develop        # enter devShell
```

### Just (preferred)

```bash
just check         # cargo check
just build         # cargo build
just clippy        # clippy with pedantic/nursery
just fmt           # cargo fmt
just test          # cargo test
just nix-check     # nix flake check
just nix-build     # nix build
just setup         # run setup script
just shellcheck    # lint shell scripts
```

## Code Standards

- All Rust code: clippy with `-W clippy::nursery -W clippy::pedantic`
- All Rust code: `cargo fmt` formatted
- All shell scripts: pass `shellcheck`
- macOS only (launchd, no systemd)
