# rustaceanvim Integration

rustaceanvim is an LSP integration. It does not provide MCP tools; use [Codex](codex.md), [Claude Code](claude-code.md), or [Generic MCP](generic-mcp.md) alongside it if you want agent-callable tools.

## Primary Config Path

Use rustaceanvim's built-in lspmux support. When `server.cmd` is unset, rustaceanvim enables lspmux auto-discovery by default and tries to connect to a running lspmux server.

For TCP loopback:

```lua
vim.g.rustaceanvim = {
  server = {
    lspmux = {
      host = '127.0.0.1',
      port = 27631,
    },
  },
}
```

Do not set `server.cmd` unless you want to bypass rustaceanvim's lspmux auto-discovery. If auto-discovery cannot find the server, set `server.lspmux.host` and `server.lspmux.port` explicitly as above.

## Transport

Use this lspmux config for the sandbox-friendly TCP path:

```toml
listen = "tcp://127.0.0.1:27631"
connect = "tcp://127.0.0.1:27631"
```

Unix sockets also work for local Neovim when the default lspmux config uses a socket path.

## Verification

Open a Rust file and run:

```vim
:checkhealth vim.lsp
```

The active client should be `rust-analyzer`. Use `:LspInfo` only if your Neovim distribution still provides it through `nvim-lspconfig`.
