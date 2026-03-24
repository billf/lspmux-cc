# Connection Troubleshooting Reference

## Socket exists but connect() fails

The most common Claude Code failure. The socket file is present at the expected path, but `connect()` is blocked by the macOS seatbelt sandbox.

**Fix:** Add the socket path to `allowUnixSockets` in `~/.claude/settings.json`:

```json
{
  "sandbox": {
    "network": {
      "allowUnixSockets": ["/var/folders/.../T/lspmux/lspmux.sock"]
    }
  }
}
```

Replace the path with your actual socket path (run `./setup doctor` to find it).

Automated fix: `./setup sandbox claude-code`

**Don't** set `allowAllUnixSockets: true`. That exposes Docker, SSH agent, gpg-agent, and other daemon sockets to the sandboxed process.

## Service not running

`./setup doctor` shows `shared socket: not ready`.

The lspmux server isn't running. Inside Claude Code's sandbox, `launchctl bootstrap` is blocked (requires `mach-bootstrap` privilege). The service must be pre-started.

**Fix:** Run `./setup core` outside of Claude Code (in a regular terminal). This deploys the launchd plist and starts the service.

## Plugin not installed

`claude plugin list` doesn't show `lspmux-rust-cc`.

**Fix:**
```bash
claude plugin add-marketplace /path/to/lspmux-cc
claude plugin install lspmux-rust-cc --scope user
```

## Built-in rust-analyzer still active

Both `rust-analyzer-lsp` and `lspmux-rust-cc` provide LSP servers. Running both causes duplicate diagnostics and non-deterministic symbol resolution.

**Fix:**
```bash
claude plugin disable rust-analyzer-lsp --scope user
```

Verify: `claude plugin list` should show `rust-analyzer-lsp` as disabled.

## MCP tools return nothing in subagents

Subagents inherit the parent's MCP connection. They don't trigger `SessionStart` hooks and don't spawn new MCP server processes.

If the parent's MCP bootstrap failed, subagents get zero MCP tools with no error message. The only diagnostic context available to subagents is the `systemMessage` from the session-start hook in conversation history.

**Fix:** The root cause is always in the parent session. Check that `rust_server_status` works in the parent conversation. If it doesn't, fix the parent connection first.

## rust-analyzer still indexing

After first connection, rust-analyzer needs time to index the workspace. Large workspaces can take 30-60 seconds (or longer for very large monorepos).

During indexing, `rust_diagnostics` may return empty results or partial data. `rust_server_status` will show the server is connected but workspace information may be incomplete.

**Fix:** Wait. There's no way to speed this up. Subsequent connections reuse the index cache.

## TCP fallback (reduced security)

If Unix socket configuration isn't possible, lspmux supports TCP localhost:

1. Edit `config.toml`: set `listen = "tcp://127.0.0.1:27631"`
2. Set `LSPMUX_SOCKET_PATH=tcp://127.0.0.1:27631` in environment

This bypasses the sandbox's Unix socket restriction because localhost TCP is allowed by default. The tradeoff: any local process can connect to the lspmux server. There's no authentication.

Use Unix sockets when possible.
