# Generic MCP Integration

Any MCP-capable host can run `lspmux-cc-mcp` over stdio. This is an MCP-only integration; it does not configure editor LSP.

## Launch Command

Manual checkout:

```sh
/absolute/path/to/lspmux-cc/bin/lspmux-cc-mcp
```

Cargo install:

```sh
lspmux-cc-mcp
```

Nix default package:

```sh
nix build
/absolute/path/to/lspmux-cc/result/bin/lspmux-cc-mcp
```

## Environment Contract

| Variable | Recommended value | Required |
|----------|-------------------|----------|
| `WORKSPACE_ROOT` | Absolute Rust workspace path | Yes |
| `LSPMUX_BOOTSTRAP` | `auto` | No |
| `LSPMUX_CONNECT` | `tcp://127.0.0.1:27631` | No |
| `LSPMUX_PATH` | Absolute path to `lspmux` when not on `PATH` | No |
| `RUST_ANALYZER_PATH` | Absolute path to `rust-analyzer` when not on `PATH` | No |
| `LSPMUX_CONFIG_PATH` | Absolute path to lspmux config | No |
| `LSPMUX_CLIENT_KIND` | Host-specific value, for example `generic_mcp` | No |
| `LSPMUX_CLIENT_HOST` | Host name, for example `my-agent` | No |
| `LSPMUX_SESSION_ID` | Stable session id | No |

`LSPMUX_BOOTSTRAP=auto` reuses a reachable daemon or starts `lspmux server` on demand. A launchd/systemd service is not required. `LSPMUX_SOCKET_PATH` remains supported as a compatibility alias, but new host configs should use `LSPMUX_CONNECT`.

## TCP Loopback Example

lspmux config:

```toml
listen = "tcp://127.0.0.1:27631"
connect = "tcp://127.0.0.1:27631"
```

Host environment:

```sh
export WORKSPACE_ROOT=/absolute/path/to/workspace
export LSPMUX_BOOTSTRAP=auto
export LSPMUX_CONNECT=tcp://127.0.0.1:27631
export LSPMUX_CLIENT_KIND=generic_mcp
export LSPMUX_CLIENT_HOST=my-agent
```

## Nix Paths

Use `nix build` for the MCP binary and `nix build .#rust-analyzer-nightly` for the pinned rust-analyzer package. Point the host at the resulting absolute paths or enter `nix develop` before launching the MCP host so `lspmux`, `lspmux-cc-mcp`, and `rust-analyzer` are on `PATH`.

## Verification

Call:

```text
rust_server_status
```

Then call:

```text
rust_diagnostics
```

with an absolute `.rs` file path in `WORKSPACE_ROOT`. A successful diagnostics response, even an empty list, confirms that MCP, `lspmux client`, `lspmux server`, and `rust-analyzer` are connected.
