---
name: diagnose-lspmux
description: Diagnose lspmux rust-analyzer connection issues. Use when MCP tools fail, rust_server_status shows errors, or rust-analyzer seems unresponsive.
disable-model-invocation: true
allowed-tools: Read, lspmux-rust-analyzer:rust_server_status, lspmux-rust-analyzer:rust_diagnostics
---

# Diagnose lspmux

Run through each step in order. Stop at the first failure and report the fix.

## Step 0: Config sanity check (run this first)

MCP tools won't work if the connection is broken, which is exactly when this skill gets invoked. Start by checking host configuration rather than assuming the current repo vendors lspmux-cc.

1. Look for repo-local host config first:
   - Claude Code: `.mcp.json`
   - Codex: `.codex/config.toml`
2. Read the active host config and find the lspmux endpoint override, if any:
   - Prefer `LSPMUX_CONNECT`
   - Fall back to `LSPMUX_SOCKET_PATH`
   - If neither is present, inspect `LSPMUX_CONFIG_PATH` or the platform-default lspmux config and read its `connect` field
3. If the resolved endpoint is a Unix socket path, read `~/.claude/settings.json` and check `sandbox.network.allowUnixSockets`. The exact socket path must be listed there.
4. If the endpoint is `tcp://...` or `host:port`, skip the Unix socket sandbox check. TCP localhost does not use `allowUnixSockets`.

If the repo being inspected is the `lspmux-cc` repo itself and `./setup` exists, you may mention `./setup doctor` as an optional follow-up verification step. Do not assume it exists in arbitrary user repos.

## Step 1: MCP health check

Call `lspmux-rust-analyzer:rust_server_status()`.

- If the call fails entirely: MCP server isn't connected. The problem is upstream (sandbox, service not running, plugin not installed). Return to Step 0 findings.
- If `server_status` is `"error"`: read the error message. Common causes: lspmux binary missing, config missing, socket unreachable.
- If `server_status` is `"ok"` and `service_mode` is `"reused"`: the shared service is working. Proceed to Step 2.
- If `service_mode` is `"started_directly"`: the shared service wasn't available and a direct instance was spawned. This works but doesn't share with other editors. Suggest running `./setup core`.

## Step 2: Pipeline test

Call `lspmux-rust-analyzer:rust_diagnostics(file_path: "<path>")` on any `.rs` file in the workspace.

- If it returns diagnostics (even empty): the full pipeline works. rust-analyzer is connected and responding.
- If it returns an error about "not indexed" or "workspace not found": rust-analyzer is still starting up. Wait 30-60 seconds and retry.
- If it returns a connection error: the lspmux client can't reach the server. Re-check the resolved endpoint from Step 0 and whether the sandbox rules match that transport.

## Step 3: Report

Summarize findings as a structured checklist:

```
lspmux diagnostic report
  Service:     [pass/fail] shared lspmux service
  Socket:      [pass/fail] socket exists and connectable
  Sandbox:     [pass/fail] allowUnixSockets configured
  Plugin:      [pass/fail] lspmux-rust-cc installed
  Built-in RA: [pass/fail] rust-analyzer-lsp disabled
  MCP:         [pass/fail] rust_server_status responds
  Pipeline:    [pass/fail] rust_diagnostics returns data
```

For any `[fail]` item, include the specific remediation command.

**Note:** `./setup core` and `./setup sandbox claude-code` are only available in the `lspmux-cc` repo. When they apply, they require filesystem and network access that Claude Code's sandbox blocks. Ask the user to run these outside the sandbox:

```
./setup core
./setup sandbox claude-code
```

## Reference

Read `reference/connection-troubleshooting.md` for detailed troubleshooting guidance on each failure mode.
