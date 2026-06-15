# Neovim LSP Integration

Neovim is an LSP integration. It does not provide MCP tools; pair it with an MCP host guide if you also want agent-callable rust-analyzer tools.

## Primary Config Path

Use Neovim's built-in LSP configuration API:

```lua
vim.lsp.config('rust_analyzer_lspmux', {
  cmd = { 'lspmux', 'client', '--server-path', 'rust-analyzer' },
  filetypes = { 'rust' },
  root_markers = { 'Cargo.toml', 'rust-project.json', '.git' },
})

vim.lsp.enable('rust_analyzer_lspmux')
```

This starts `lspmux client` as the language server process. The client connects to the shared `lspmux server`, which starts `rust-analyzer` on demand.

## Optional nvim-lspconfig Override

If your configuration is already centered on `nvim-lspconfig`, override only the command:

```lua
require('lspconfig').rust_analyzer.setup({
  cmd = { 'lspmux', 'client', '--server-path', 'rust-analyzer' },
})
```

Do not enable both the built-in config above and a separate `nvim-lspconfig` `rust_analyzer` setup for the same buffers.

## Transport

For TCP loopback:

```sh
export LSPMUX_CONNECT=tcp://127.0.0.1:27631
```

For Unix sockets, leave `LSPMUX_CONNECT` unset and let `lspmux client` read the configured `connect` value.

## Verification

Open a Rust file and run:

```vim
:checkhealth vim.lsp
```

Check that a `rust_analyzer_lspmux` or `rust_analyzer` client is attached.
