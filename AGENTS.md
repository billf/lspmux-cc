# lspmux-cc

LSP multiplexing for Claude Code via [lspmux-rust-analyzer](https://github.com/sunshowers/lspmux-rust-analyzer).

This file is the canonical contributor guide. It is read by Codex (walked from repo root to
cwd), OpenCode, and other agent runtimes. `CLAUDE.md` defers to this file so the guidance
cannot drift between runtimes.

## Project Layout

- Shell-script-first project with a Rust sub-project under `mcp-server/`
- Plugin files at repo root: `.claude-plugin/`, `.mcp.json`, `.lsp.json`, `bin/`, `hooks/`, `skills/`
- Nix flake provides devShell, packages (including `plugin`), and checks
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
just check         # cargo check (asserts toolchain pin first)
just build         # cargo build (asserts toolchain pin first)
just clippy        # clippy with pedantic/nursery (asserts toolchain pin first)
just fmt           # cargo fmt
just test          # cargo test (asserts toolchain pin first)
just nix-check     # nix flake check
just nix-build     # nix build
just setup         # run setup script
just shellcheck    # lint shell scripts
```

`rust-toolchain.toml` pins the interactive toolchain to the flake's nixpkgs
rustc; `just` recipes (except `fmt`) refuse to run when ambient cargo
disagrees; inside `nix develop`, they verify the pinned rustc version and cargo train. See `docs/binary-resolution.md`.

## Code Standards

- All Rust code: clippy with `-W clippy::nursery -W clippy::pedantic`
- All Rust code: `cargo fmt` formatted
- All shell scripts: pass `shellcheck`
- macOS-first; daemons spawned per-workspace on demand by the MCP server (no system service manager required)

## Using the MCP tools (any runtime)

The MCP server exposes 14 read-only `rust_*` tools (diagnostics, hover, goto, references,
workspace/document symbols, code actions, rename, call hierarchy, implementation, macro
expansion, server status, registry). These rules matter when calling them:

- **Coordinates are zero-based in, one-based out.** Tool `line`/`character` inputs are
  zero-based (first line = 0). Returned `file:line:col` locations are one-based. Subtract 1
  from a returned location before feeding it back as input.
- **Tools never write to disk.** `rust_rename` and `rust_code_actions` return a workspace
  edit as data; apply the edits yourself. Every tool is read-only.

- **References and workspace symbols are capped.** `max_results` defaults to 200,
  clamps values below 1 to 1, and has a hard cap of 1000. Check `truncated` and
  `total_count`; the cap limits returned results, not the full query's work.
- **Workspace symbol queries must not be blank.** Blank queries return
  `invalid_params`; pass a narrowed substring.
- **Indexing failures can be transient.** Check `rust_server_status` and retry
  after rust-analyzer finishes indexing.

The server also ships these rules in its MCP `instructions` string, so an agent that only
calls the server sees them without reading this file.
