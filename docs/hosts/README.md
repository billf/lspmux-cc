# Host and Editor Integrations

Use one `lspmux server` per machine session and point each host or editor at it. Agent hosts usually use MCP, editors usually use LSP, and Claude Code can use both.

For sandboxed agent hosts, prefer `LSPMUX_CONNECT=tcp://127.0.0.1:27631`. Unix sockets are still supported when the host can connect to the socket path.

| Host | Integration type | Config surface | Transport recommendation | Verification command |
|------|------------------|----------------|--------------------------|----------------------|
| [Claude Code](claude-code.md) | LSP and MCP | Claude plugin plus `~/.claude/settings.json` sandbox settings | TCP loopback for sandbox simplicity; Unix socket with exact `allowUnixSockets` allowlist | `rust_server_status` in Claude Code |
| [Codex](codex.md) | MCP | `~/.codex/config.toml` or `.codex/config.toml` | TCP loopback | `rust_server_status` in Codex |
| [Generic MCP](generic-mcp.md) | MCP | Host MCP command/env configuration | TCP loopback | `rust_server_status` in the host |
| [rustaceanvim](rustaceanvim.md) | LSP | `vim.g.rustaceanvim` | TCP loopback | `:checkhealth vim.lsp` |
| [Neovim LSP](neovim.md) | LSP | `vim.lsp.config` / `vim.lsp.enable` | TCP loopback or Unix socket | `:checkhealth vim.lsp` |
| [Vim/vim-lsp](vim-lsp.md) | LSP | `lsp#register_server` | TCP loopback or Unix socket | `:LspStatus` |
| [VS Code](vscode.md) | LSP | `rust-analyzer.server.path` | Unix socket or TCP through a wrapper | `rust-analyzer` output channel |

## Troubleshooting (any MCP host)

These steps use only the MCP tools, so they work from any MCP host (Claude Code, Codex, or a
generic MCP host); the `diagnose-lspmux` skill wraps the same steps for Claude Code
specifically.

1. **Check liveness and workspace.** Call `rust_server_status`. Healthy: `server_status` is
   `running` and `workspace_root` is your workspace. `readiness.health` becomes `ok` once
   indexing finishes; until then, navigation calls may return empty or a "still indexing"
   retry hint.
2. **Confirm the workspace matches.** Call `rust_workspace_registry` and check that an
   instance's `workspace_root` matches yours. A mismatch means the daemon is serving a
   different workspace; set `WORKSPACE_ROOT` in your host's MCP env and reconnect.
3. **Exercise the full path.** Call `rust_diagnostics` on an absolute path to a `.rs` file. A
   result (even an empty one) confirms the MCP-to-LSP path works end to end.
4. **If calls fail transiently**, the error says to check `rust_server_status` and retry:
   rust-analyzer is likely still indexing. Wait for `readiness.health` to reach `ok`.
5. **If the daemon is unreachable**, verify the transport. Sandboxed hosts should set
   `LSPMUX_CONNECT=tcp://127.0.0.1:27631`; the `lspmux` config's `listen`/`connect` must
   agree. Unix-socket hosts need the socket path reachable.

## Names

Four names refer to parts of one project; they intentionally differ:

- `lspmux-cc` — the repository and the marketplace.
- `lspmux-rust-cc` — the Claude Code plugin (`.claude-plugin/plugin.json`).
- `lspmux-rust-analyzer` — the MCP server key, so tool ids read as
  `mcp__lspmux-rust-analyzer__rust_*`. This name is part of every host's config
  (`.codex/config.toml`, `opencode.json`, `.mcp.json`), so it is kept stable: renaming it
  would break existing host configurations.
- `rust-analyzer` — the underlying language server, wrapped unchanged.
