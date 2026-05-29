# Plan: Claude Code Connection Documentation, Sandboxing, and Verification

**Status:** Plan, deepened 2026-03-23. Predates the current repo-root plugin
layout, so the `plugins/lspmux-rust-cc/...` paths below now live at the repo
root (`bin/`, `hooks/`, `.mcp.json`, etc.). Execution status not audited; treat
the deliverables as a checklist, not a record of completed work.

## Context

lspmux-cc's Claude Code integration works (plugin, hooks, MCP server, LSP routing) but the documentation doesn't cover the full connection story. Users need to understand: how to connect, how sandboxing affects the setup, how to verify everything works, and how to override Claude's built-in rust-analyzer.

This is preventive work. The plugin will be used with Claude Code agents/subagents and Codex, where a lot can go wrong silently. The audience is both new users (onboarding) and existing users (troubleshooting).

**Deepened on:** 2026-03-23
**Research agents used:** best-practices-researcher (x3: macOS sandbox + Unix sockets, Claude plugin CLI + lifecycle, MCP verification patterns)

## Critical Research Findings

### 1. macOS Sandbox BLOCKS Unix socket `connect()` by default

Unix domain socket `connect()` is controlled by `network-outbound` rules in the seatbelt profile, NOT by `file-read-data`. Having read-only filesystem access to the socket path lets you `stat()` it, but connecting requires an explicit `(allow network-outbound (remote unix-socket (literal "/tmp/lspmux/lspmux.sock")))` rule.

Claude Code's sandbox schema has **explicit Unix socket allowlisting**:
```json
{
  "sandbox": {
    "network": {
      "allowUnixSockets": [],
      "allowAllUnixSockets": false
    }
  }
}
```

**This means: users MUST add the lspmux socket path to `allowUnixSockets` in their Claude Code settings.** Without this, the MCP server and LSP client both fail to connect to the lspmux Unix socket.

**Workaround options (ranked):**
1. **Best:** `allowUnixSockets: ["/tmp/lspmux/lspmux.sock"]` in Claude Code's `settings.json` (purpose-built for this)
2. **Good fallback:** Add TCP localhost support (lspmux already supports `listen = "tcp://127.0.0.1:27631"`; the sandbox allows localhost TCP)
3. **Workspace-local:** Put socket inside CWD (sandbox allows writes there; breaks multi-project multiplexing)

### 2. `launchctl bootstrap` is blocked inside sandbox

Requires `mach-bootstrap` operation, only granted in the base `system.sb`. Application-level profiles don't get this. The MCP server's `try_start_via_manager()` will always fail silently under sandbox. The `start_direct_server()` fallback also fails (can't create socket files outside CWD).

**Implication:** `LSPMUX_BOOTSTRAP=require` is the correct default for the plugin. The service MUST be pre-started outside the sandbox.

### 3. Plugin CLI and identifiers confirmed

- Built-in rust-analyzer plugin ID: `rust-analyzer-lsp` (from official Anthropic marketplace)
- Commands: `claude plugin install|uninstall|enable|disable|list|validate`
- `claude plugin list --json` returns `id`, `scope`, `enabled`, `version`, `install_path`
- `/reload-plugins` picks up changes without restart
- Scopes: `user` (~/.claude/settings.json), `project` (.claude/settings.json), `local` (.claude/settings.local.json)

### 4. Subagent MCP lifecycle

- **Subagents inherit parent's MCP server connections** (no new MCP server processes spawned)
- Subagents do NOT trigger `SessionStart` hooks (they fire `SubagentStart`/`SubagentStop` instead)
- Subagents inherit all parent tools unless restricted via `tools`/`disallowedTools` frontmatter
- Subagents can declare their own `mcpServers` in frontmatter (inline definitions start new processes)

### 5. Codex uses TOML config, not `.mcp.json`

- Config: `~/.codex/config.toml` or `.codex/config.toml` (project)
- Each server: `[mcp_servers.<name>]` table with `command` and `args`
- No plugin system, no LSP support
- Sandbox modes: `read-only`, `workspace-write`, `danger-full-access`

### 6. MCP error handling best practice

During degraded state (RA indexing, service starting up), tools should return `CallToolResult { isError: true }` with descriptive content, NOT protocol-level `McpError`. The LLM can reason about `isError` responses and retry; protocol errors look like crashes.

## Deliverables

### 1. Rewrite `docs/hosts/claude-code.md`

**File:** `docs/hosts/claude-code.md` (35 → ~250 lines)

**Structure:**

1. **Overview** (3 lines) — two channels: LSP (native Rust support) + MCP (agent tools). Both are sandboxed child processes.

2. **Prerequisites** — `./setup core` must run BEFORE plugin install. The launchd service must be running because the sandbox blocks service startup from child processes.

3. **Sandbox Configuration** (this is the critical new section)
   - Claude Code's seatbelt sandbox blocks Unix socket `connect()` by default
   - Users must add the socket path to `allowUnixSockets` in settings:
     ```json
     {
       "sandbox": {
         "network": {
           "allowUnixSockets": ["/tmp/lspmux/lspmux.sock"]
         }
       }
     }
     ```
   - Explain where to find the actual socket path: `./setup doctor` prints it, or check `LSPMUX_SOCKET_PATH` env var
   - Platform-specific socket paths: macOS uses `$TMPDIR/lspmux/lspmux.sock`, Linux uses `$XDG_RUNTIME_DIR/lspmux/lspmux.sock`
   - TCP localhost alternative: if socket allowlisting is too fragile, configure lspmux to listen on TCP and set `LSPMUX_SOCKET_PATH` accordingly

4. **Install** — 3 `claude plugin` commands with explanation:
   - `claude plugin marketplace add /absolute/path/to/lspmux-cc` — registers marketplace from `.claude-plugin/marketplace.json`
   - `claude plugin disable rust-analyzer-lsp --scope user` — disables official Anthropic RA plugin. Only one LSP per language should be active. Confirm with `claude plugin list`.
   - `claude plugin install lspmux-rust-cc --scope user` — installs plugin (LSP + MCP + hooks). Use `--scope project` if sharing via VCS.

5. **How the Connection Works** — mermaid diagram showing:
   ```
   Claude Code → plugin.json lspServers → bin/lspmux → lspmux client → Unix socket → lspmux server (launchd) → rust-analyzer
   Claude Code → .mcp.json stdio → bin/lspmux-cc-mcp → lspmux-cc-mcp binary → spawns lspmux client → same Unix socket
   ```

6. **Overriding Built-in rust-analyzer**
   - The official plugin is `rust-analyzer-lsp` from the Anthropic marketplace
   - `claude plugin disable rust-analyzer-lsp --scope user` stops it from registering at startup
   - Verify: `claude plugin list` (or `/plugin` > Installed tab)
   - If both are active simultaneously: behavior is non-deterministic (last-loaded wins). Always disable one before enabling the other.
   - To revert: `claude plugin enable rust-analyzer-lsp --scope user` and `claude plugin disable lspmux-rust-cc --scope user`

7. **Verification** — three layers:
   - **Before Claude Code:** `./setup doctor` (checks binaries, config, socket, service)
   - **Inside session:** call `rust_server_status` MCP tool. Check: `server_status: "running"`, `runtime.service_mode: "reused"`, `workspace_root` matches
   - **LSP verification:** `Ctrl+O` for diagnostics, `/plugin` for plugin status, debug logs at `~/.claude/debug/latest` (search for "Total LSP servers loaded")

8. **Agent and Subagent Usage**
   - Subagents inherit the parent's MCP server connection (no new process spawned)
   - Subagents do NOT trigger `SessionStart` hooks
   - All 6 MCP tools available to subagents unless restricted via `disallowedTools`
   - Common failure: subagent can't call tools because parent's MCP server failed to connect at session start

9. **Troubleshooting** — symptom table:

   | Symptom | Cause | Fix |
   |---------|-------|-----|
   | MCP tools return connection error | Socket not in `allowUnixSockets` | Add socket path to Claude Code sandbox settings |
   | `rust_server_status` returns "stopped" | Service not running | Run `./setup core` outside Claude Code |
   | `service_mode: "started_directly"` | Sandbox allowed direct spawn (unusual) | Fine, but prefer managed service via `./setup core` |
   | Bootstrap failure error | `LSPMUX_BOOTSTRAP=require` and service not running | Run `./setup core`, verify with `./setup doctor` |
   | LSP completions missing | `rust-analyzer-lsp` still active, or plugin not installed | Check `/plugin`, disable official, install lspmux-rust-cc |
   | Stale diagnostics after edit | RA re-indexing | Wait, check `rust_server_status` readiness |
   | `lspmux not found` | Binary not on PATH | Set `LSPMUX_PATH` or run `./setup core` |

10. **Environment Variables** — table with all vars, defaults, descriptions. Note which are set automatically by the plugin vs overridable.

### 2. Update `.mcp.json` to set `LSPMUX_BOOTSTRAP=require`

**File:** `plugins/lspmux-rust-cc/.mcp.json`

Add `"LSPMUX_BOOTSTRAP": "require"` to the env block. Under sandbox, `auto` mode's fallbacks (launchctl, direct spawn) silently fail. `require` makes the failure loud: "shared lspmux service is unavailable; run `./setup core`".

```json
{
  "mcpServers": {
    "lspmux-rust-analyzer": {
      "command": "${CLAUDE_PLUGIN_ROOT}/bin/lspmux-cc-mcp",
      "env": {
        "RUST_ANALYZER_PATH": "${CLAUDE_PLUGIN_ROOT}/bin/rust-analyzer",
        "LSPMUX_CLIENT_KIND": "claude_mcp",
        "LSPMUX_CLIENT_HOST": "claude",
        "LSPMUX_BOOTSTRAP": "require"
      }
    }
  }
}
```

### 3. Add `allowUnixSockets` guidance to `./setup host claude-code`

**File:** `setup` (`cmd_host_claude_code` function, lines 140-153)

Expand the output to include:
- Prerequisite check: warn if socket isn't ready
- The `allowUnixSockets` configuration step (the critical sandbox fix)
- Verification: `./setup doctor`
- Link to full guide

### 4. Enhance `./setup doctor` with Claude-specific checks

**File:** `setup` (`cmd_doctor` function, lines 190-243)

Add checks (gated on `claude` being available on PATH):
- `claude plugin list` to check `lspmux-rust-cc` is installed
- `claude plugin list` to check `rust-analyzer-lsp` is disabled
- Check if socket path is in Claude's `allowUnixSockets` (read `~/.claude/settings.json` with jq)
- If `claude` not found, skip with a note

### 5. Create diagnostic skill

**File:** `plugins/lspmux-rust-cc/skills/diagnose-lspmux/SKILL.md` (new)

Structure (following Anthropic's skill authoring best practices):
- Progressive disclosure: quick check first, deep-dive in reference files
- Checklist pattern: Claude copies and works through steps
- Use fully qualified tool names: `lspmux-rust-analyzer:rust_server_status()`

Content:
1. Call `rust_server_status` — check status, service_mode, workspace_root
2. If status != "running": check socket path, suggest `./setup core`, check `allowUnixSockets`
3. If status == "running": call `rust_diagnostics` on a `.rs` file to confirm full pipeline
4. If diagnostics fail: check RA readiness (quiescent), suggest waiting
5. Report summary with remediation steps

Reference files in `skills/diagnose-lspmux/reference/`:
- `sandbox-troubleshooting.md` — socket allowlisting, TCP fallback
- `bootstrap-modes.md` — auto vs require vs off under sandbox

### 6. Improve session-start hook messages

**File:** `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh`

When service is running (line 27): add "Use rust_server_status() to confirm workspace root and readiness."

When service is NOT running and bootstrap is `auto` (line 37): add sandbox warning: "Claude Code's sandbox prevents starting services. If bootstrap fails, exit Claude Code, run `./setup core`, and restart."

### 7. Update README.md Claude Code section

**File:** `README.md` (Host Integrations > Claude Code)

Add:
- "Run `./setup core` first (starts the shared service outside the sandbox)"
- The `allowUnixSockets` configuration requirement
- Link to `docs/hosts/claude-code.md`

### 8. Update `docs/hosts/codex.md`

**File:** `docs/hosts/codex.md`

Add:
- Codex uses TOML config (`~/.codex/config.toml`), not `.mcp.json`
- Example `[mcp_servers.lspmux-rust-analyzer]` block
- Codex sandbox modes and which to use
- No LSP support (MCP tools only)

## Execution Order

1. `.mcp.json` — add `LSPMUX_BOOTSTRAP=require` (small change, immediate safety win)
2. `docs/hosts/claude-code.md` — full rewrite (everything else references it)
3. `setup` — update `cmd_host_claude_code` and `cmd_doctor`
4. `session-start.sh` — improve messages
5. Diagnostic skill — new SKILL.md + reference files
6. `README.md` — update Claude Code section
7. `docs/hosts/codex.md` — update with TOML config format

## Verification

- `just shellcheck` passes (setup, session-start.sh modified)
- `./setup doctor` runs and shows new Claude-specific checks
- `./setup host claude-code` shows sandbox configuration steps
- Read through `docs/hosts/claude-code.md` for completeness
- Manual: install plugin, add `allowUnixSockets`, call `rust_server_status`, run diagnostic skill

## Files Modified

| File | Change |
|------|--------|
| `plugins/lspmux-rust-cc/.mcp.json` | Add `LSPMUX_BOOTSTRAP=require` |
| `docs/hosts/claude-code.md` | Rewrite (35 → ~250 lines) |
| `setup` | Enhance `cmd_doctor`, update `cmd_host_claude_code` |
| `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh` | Improve sandbox-aware messages |
| `plugins/lspmux-rust-cc/skills/diagnose-lspmux/SKILL.md` | New diagnostic skill |
| `plugins/lspmux-rust-cc/skills/diagnose-lspmux/reference/sandbox-troubleshooting.md` | New reference |
| `plugins/lspmux-rust-cc/skills/diagnose-lspmux/reference/bootstrap-modes.md` | New reference |
| `README.md` | Update Claude Code section |
| `docs/hosts/codex.md` | Update with TOML config format |
