# Vim/vim-lsp Integration

`prabirshrestha/vim-lsp` is an LSP integration for Vim 8 and Neovim. It does not provide MCP tools.

## Primary Config Path

Register a Rust server with `lsp#register_server`:

```vim
if executable('lspmux')
  au User lsp_setup call lsp#register_server({
      \ 'name': 'rust-analyzer-lspmux',
      \ 'cmd': {server_info->['lspmux', 'client', '--server-path', 'rust-analyzer']},
      \ 'allowlist': ['rust'],
      \ })
endif
```

The `cmd` launches `lspmux client` and passes `rust-analyzer` as the language server path for the shared daemon to spawn.

## Transport

For TCP loopback:

```sh
export LSPMUX_CONNECT=tcp://127.0.0.1:27631
```

For Unix sockets, leave `LSPMUX_CONNECT` unset and let `lspmux client` read the configured `connect` value.

## Verification

Open a Rust file and run:

```vim
:LspStatus
```

The registered `rust-analyzer-lspmux` server should be running for the buffer.
