# Generic MCP Integration

Any MCP-capable host can use `lspmux-cc-mcp` directly.

## Requirements

- A user-level `lspmux` service installed via `./setup core`
- A `WORKSPACE_ROOT` pointing at the active Rust workspace
- `rust-analyzer` available via `RUST_ANALYZER_PATH` or on `PATH`

## Environment Contract

- `WORKSPACE_ROOT`
- `LSPMUX_BOOTSTRAP=auto|require|off`
- `LSPMUX_PATH`
- `RUST_ANALYZER_PATH`
- `LSPMUX_CONFIG_PATH`
- `LSPMUX_SOCKET_PATH`
- `LSPMUX_CLIENT_KIND`
- `LSPMUX_CLIENT_HOST`
- `LSPMUX_SESSION_ID`

`LSPMUX_BOOTSTRAP=auto` is the default and will reuse the shared service when available, then fall back to a direct foreground `lspmux server` process if no managed user service is ready.

## Launch Command

```bash
plugins/lspmux-rust-cc/bin/lspmux-cc-mcp
```
