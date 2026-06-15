# VS Code Integration

VS Code uses the rust-analyzer extension as an LSP client. This guide routes that extension through `lspmux client`. It does not provide MCP tools; use an MCP host guide separately for agent-callable tools.

## Primary Config Path

The rust-analyzer extension exposes `rust-analyzer.server.path`, which points at the server executable. Set it to a wrapper script that launches `lspmux client`.

Example wrapper:

```sh
#!/usr/bin/env sh
exec lspmux client --server-path rust-analyzer "$@"
```

Make it executable and point VS Code at it:

```json
{
  "rust-analyzer.server.path": "/absolute/path/to/rust-analyzer-lspmux"
}
```

The VS Code setting is a path to an executable, not a full command line. If your extension version or environment cannot pass `lspmux client --server-path ...` arguments directly, keep the tiny wrapper and put the arguments there.

## Transport

The wrapper inherits VS Code's environment.

For TCP loopback, launch VS Code from a shell that exports:

```sh
export LSPMUX_CONNECT=tcp://127.0.0.1:27631
code .
```

For Unix sockets, leave `LSPMUX_CONNECT` unset and let `lspmux client` read the configured `connect` value.

## Verification

Open a Rust file, run `Output: Show Output Channels`, select `rust-analyzer`, and confirm the server starts without wrapper or connection errors.
