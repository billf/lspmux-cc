---
name: diagnose-lspmux
description: Diagnose lspmux rust-analyzer connection issues. Use when MCP tools fail, rust_server_status shows errors, or rust-analyzer seems unresponsive.
disable-model-invocation: true
allowed-tools: Read, Bash(./setup *), lspmux-rust-analyzer:rust_server_status, lspmux-rust-analyzer:rust_diagnostics
---

# Diagnose lspmux

Run through each step in order. Stop at the first failure and report the fix.

## Step 0: Bash fallback (run this first)

MCP tools won't work if the connection is broken, which is exactly when this skill gets invoked. Start with Bash.

1. Run `./setup doctor` via Bash. If it reports failures, guide the user through fixes before attempting MCP tools.
2. Check the socket directly: `ls -la $LSPMUX_SOCKET_PATH` (the path is printed by `./setup doctor`).
3. Check sandbox config: read `~/.claude/settings.json` and look for `sandbox.network.allowUnixSockets`. The socket path must be listed there.

If `./setup doctor` shows all `[ok]`, proceed to Step 1.

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
- If it returns a connection error: the lspmux client can't reach the server. Check `./setup doctor` output.

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

## Reference

Read `reference/connection-troubleshooting.md` for detailed troubleshooting guidance on each failure mode.
