# Claude Code Integration

`lspmux-cc` exposes a shared `rust-analyzer` instance to Claude Code in two ways:

- Claude's Rust LSP traffic goes through the `lspmux-rust-cc` plugin.
- Agent tool calls go through `lspmux-cc-mcp`.

## Install

```bash
./setup core
claude plugin add-marketplace /absolute/path/to/lspmux-cc
claude plugin disable rust-analyzer-lsp --scope user
claude plugin install lspmux-rust-cc --scope user
```

## Runtime Contract

Set these environment variables on the MCP process if you need explicit paths:

- `WORKSPACE_ROOT`
- `LSPMUX_BOOTSTRAP=auto|require|off`
- `LSPMUX_PATH`
- `RUST_ANALYZER_PATH`
- `LSPMUX_CONFIG_PATH`
- `LSPMUX_SOCKET_PATH`

The Claude MCP entry stamps `LSPMUX_CLIENT_KIND=claude_mcp` and `LSPMUX_CLIENT_HOST=claude` by default. The Claude LSP wrapper stamps `claude_lsp` / `claude`.

Hooks are informational only. They report whether a shared service is already running, but they do not start or manage services.

The Rust MCP runtime is the only bootstrap authority for launchd/systemd/direct fallback behavior. Use `rust_server_status` after MCP startup to confirm bootstrap mode and rust-analyzer readiness.

The MCP server rereads files from disk before each request, so tool correctness does not depend on Claude hooks firing.
