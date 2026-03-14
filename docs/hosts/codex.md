# Codex Integration

Codex uses `lspmux-cc` as a plain MCP server. Editors can keep using native LSP so long as they point at the same underlying `lspmux` service.

## Install

```bash
./setup core
./setup host codex
```

## Runtime Contract

Export these variables for the MCP process:

```bash
export WORKSPACE_ROOT=/absolute/path/to/workspace
export LSPMUX_BOOTSTRAP=auto
export LSPMUX_PATH="$HOME/.cargo/bin/lspmux"
export RUST_ANALYZER_PATH="$HOME/.local/share/lspmux-rust-analyzer/current/rust-analyzer"
export LSPMUX_CONFIG_PATH="$HOME/.config/lspmux/config.toml"
export LSPMUX_SOCKET_PATH="${TMPDIR:-/tmp}/lspmux/lspmux.sock"
```

On macOS, the default `LSPMUX_CONFIG_PATH` is:

```bash
$HOME/Library/Application Support/lspmux/config.toml
```

Launch the server with:

```bash
plugins/lspmux-rust-cc/bin/lspmux-cc-mcp
```

## Tool Surface

The MCP tool contract is intentionally Rust-specific and stable:

- `rust_diagnostics`
- `rust_hover`
- `rust_goto_definition`
- `rust_find_references`
- `rust_workspace_symbol`
- `rust_server_status`
