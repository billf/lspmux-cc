# Codex Integration

Codex uses `lspmux-cc` as a plain MCP server. It does not use the LSP plugin path; editors can keep using their own LSP clients as long as they point at the same `lspmux server`.

## Install

Manual checkout:

```sh
./setup core
./setup host codex
```

Nix:

```sh
nix build
nix build .#rust-analyzer-nightly
```

Use the built `result/bin/lspmux-cc-mcp` or place the flake packages in your environment. For a Claude-plugin-shaped Nix output, use `nix build .#plugin`; Codex only needs the MCP binary and environment.

## Primary Config Path

Codex uses TOML config:

- User-level: `~/.codex/config.toml`
- Project-level: `.codex/config.toml`

TCP loopback example:

```toml
[mcp_servers.lspmux-rust-analyzer]
command = "/absolute/path/to/lspmux-cc/bin/lspmux-cc-mcp"
args = []

[mcp_servers.lspmux-rust-analyzer.env]
WORKSPACE_ROOT = "/absolute/path/to/workspace"
LSPMUX_BOOTSTRAP = "auto"
LSPMUX_CONNECT = "tcp://127.0.0.1:27631"
LSPMUX_CLIENT_KIND = "codex_mcp"
LSPMUX_CLIENT_HOST = "codex"
```

For a Nix-built default package, set `command` to the built binary:

```toml
command = "/absolute/path/to/result/bin/lspmux-cc-mcp"
```

If you want the pinned rust-analyzer from this flake, export or configure:

```sh
export RUST_ANALYZER_PATH=/absolute/path/to/rust-analyzer-nightly/bin/rust-analyzer
```

## Runtime Contract

| Variable | Recommended value | Notes |
|----------|-------------------|-------|
| `WORKSPACE_ROOT` | Absolute Rust workspace path | Used for rust-analyzer initialization. |
| `LSPMUX_BOOTSTRAP` | `auto` | Reuses a reachable daemon or starts one on demand. |
| `LSPMUX_CONNECT` | `tcp://127.0.0.1:27631` | Preferred sandbox-friendly endpoint. |
| `LSPMUX_PATH` | Optional absolute path | Needed only if `lspmux` is not on `PATH`. |
| `RUST_ANALYZER_PATH` | Optional absolute path | Use this for pinned Nix rust-analyzer or a custom binary. |
| `LSPMUX_CONFIG_PATH` | Optional config path | Defaults to the platform lspmux config path. |
| `LSPMUX_CLIENT_KIND` | `codex_mcp` | Telemetry identity. |
| `LSPMUX_CLIENT_HOST` | `codex` | Telemetry host. |
| `LSPMUX_SESSION_ID` | Any stable session id | Optional; generated if omitted. |

`LSPMUX_SOCKET_PATH` is still accepted as a compatibility alias for older configs, but use `LSPMUX_CONNECT` for new Codex setups.

## Transport

For TCP loopback, set the lspmux config:

```toml
listen = "tcp://127.0.0.1:27631"
connect = "tcp://127.0.0.1:27631"
```

For Unix sockets, omit `LSPMUX_CONNECT` or set it to the absolute socket path. TCP is usually simpler in sandboxed Codex sessions.

## Verification

In a Codex session, call:

```text
rust_server_status
```

Healthy signals:

- `server_status` is `running`
- `workspace_root` is the configured workspace
- `readiness.health` becomes `ok` after indexing

Then run `rust_diagnostics` on an absolute Rust file path to verify the full MCP-to-LSP path.
