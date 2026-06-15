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
