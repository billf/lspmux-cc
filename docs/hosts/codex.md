# Codex Integration

Codex uses `lspmux-cc` as a plain MCP server. Editors can keep using native LSP so long as they point at the same underlying `lspmux` service.

## Install

```bash
./setup core
./setup host codex
```

`./setup core` validates that `rust-analyzer` is already available through
`RUST_ANALYZER_PATH` or `PATH`; it does not download the binary.

## Runtime Contract

Export these variables for the MCP process:

```bash
export WORKSPACE_ROOT=/absolute/path/to/workspace
export LSPMUX_BOOTSTRAP=auto
export LSPMUX_PATH="$HOME/.cargo/bin/lspmux"
export RUST_ANALYZER_PATH="$(command -v rust-analyzer)"
export LSPMUX_CONFIG_PATH="$HOME/.config/lspmux/config.toml"
export LSPMUX_CONNECT="${TMPDIR:-/tmp}/lspmux/lspmux.sock"
export LSPMUX_CLIENT_KIND="codex_mcp"
export LSPMUX_CLIENT_HOST="codex"
export LSPMUX_SESSION_ID="codex-$(date +%s)-$$"
```

`LSPMUX_CONNECT` accepts either a Unix socket path or a TCP endpoint like
`tcp://127.0.0.1:27631`. `LSPMUX_SOCKET_PATH` is still accepted as a
compatibility alias, but `LSPMUX_CONNECT` is the explicit name going forward.

For reproducible Nix setups, prefer exporting `RUST_ANALYZER_PATH` from this
flake's pinned package, for example via `nix build .#rust-analyzer` or a dev
shell/Home Manager environment that places the binary on `PATH`.

On macOS, the default `LSPMUX_CONFIG_PATH` is:

```bash
$HOME/Library/Application Support/lspmux/config.toml
```

Launch the server with:

```bash
bin/lspmux-cc-mcp
```

## Tool Surface

The MCP tool contract is intentionally Rust-specific and stable:

- `rust_diagnostics`
- `rust_hover`
- `rust_goto_definition`
- `rust_find_references`
- `rust_workspace_symbol`
- `rust_server_status`

## Native TOML Configuration

Codex uses TOML config files, not `.mcp.json`. You can configure lspmux-cc directly in Codex's config instead of using environment variables.

**User-level:** `~/.codex/config.toml`
**Project-level:** `.codex/config.toml`

```toml
[mcp_servers.lspmux-rust-analyzer]
command = "/absolute/path/to/lspmux-cc/bin/lspmux-cc-mcp"
args = []

[mcp_servers.lspmux-rust-analyzer.env]
WORKSPACE_ROOT = "/absolute/path/to/workspace"
LSPMUX_BOOTSTRAP = "auto"
LSPMUX_CLIENT_KIND = "codex_mcp"
LSPMUX_CLIENT_HOST = "codex"
```

Replace paths with your actual install locations.

## Sandbox Modes

Codex supports three sandbox modes: `read-only`, `workspace-write`, and `danger-full-access`.

For lspmux-cc, use `workspace-write`. The MCP server needs to read Rust source files in the workspace but doesn't write anything. `read-only` works too, since the MCP server only reads files and communicates over the Unix socket.

Codex doesn't support LSP plugins. Only the 6 MCP tools are available.
