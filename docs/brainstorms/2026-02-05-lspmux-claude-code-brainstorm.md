# lspmux-cc: LSP Multiplexing for Claude Code

**Date:** 2026-02-05
**Status:** Brainstorm

## Problem

Multiple tools in a single Rust worktree (Neovim + Claude Code sessions) each spawn their own rust-analyzer instance. These instances compete for the cargo build lock, causing build thrashing - one builds, another waits, invalidates, rebuilds. This wastes CPU, memory, and developer time.

Direct cargo commands (build/check/clippy/test) are handled by cargo's built-in locking, so the primary problem is duplicate rust-analyzer instances.

## What We're Building

A Claude Code integration layer around [lspmux-rust-analyzer](https://github.com/sunshowers/lspmux-rust-analyzer), which uses the [lspmux crate](https://docs.rs/lspmux/0.3.0/lspmux/) to share a single rust-analyzer instance across all LSP clients.

Three components:

### 1. Auto-setup script for Claude Code
Automates the full setup:
- Install lspmux binary (via cargo install)
- Download rust-analyzer
- Configure launchd service on macOS
- Configure Claude Code plugin (disable built-in rust-analyzer, enable lspmux)
- Configure Neovim lspconfig to use lspmux client
- Verify everything works (connectivity test)

### 2. Claude Code MCP server
An MCP server that exposes rust-analyzer capabilities as tools Claude Code can invoke directly:
- Get diagnostics for a file/workspace
- Request completions at a position
- Request hover information
- Trigger code actions / refactoring
- Run workspace-wide diagnostics
- This goes beyond the LSP plugin by giving Claude Code's agent explicit tool access to rust-analyzer intelligence

### 3. Claude Code hooks for coordination
Hooks that:
- Ensure Claude Code's LSP requests route through lspmux
- Coordinate around build events (e.g., after a file save, wait for rust-analyzer to re-check before running cargo test)
- Surface rust-analyzer diagnostics proactively

## Why This Approach

- **lspmux is proven** - The lspmux crate already handles the hard LSP multiplexing protocol work
- **lspmux-rust-analyzer provides the recipe** - Shell scripts, service configs, and editor configs already exist
- **Claude Code integration is the gap** - The existing setup has basic Claude Code plugin config but no deep integration (MCP tools, hooks, auto-setup)
- **Incremental** - Each component is independently useful. Auto-setup alone is valuable. MCP server adds intelligence. Hooks add coordination.

## Key Decisions

1. **Wrap lspmux-rust-analyzer, don't fork** - Use it as the foundation, extend for Claude Code
2. **macOS first** (launchd) - User's platform. Linux (systemd) can come later
3. **Neovim as the editor** - Alongside Claude Code. nvim-lspconfig integration
4. **Single rust-analyzer per workspace** - lspmux's model. One server process, multiple client connections via TCP (localhost:27631)
5. **Build order: setup -> hooks -> MCP** - Foundation first, then coordination, then rich tooling

## Architecture

```
Neovim ──── lspmux client ──┐
                             ├── TCP ── lspmux server ── rust-analyzer
Claude Code ─ lspmux client ─┘            (launchd)      (one per workspace)
                │
                ├── MCP server (tools for diagnostics, completions, etc.)
                └── Hooks (coordination, proactive diagnostics)
```

## Open Questions

- Should the MCP server talk to rust-analyzer directly or through lspmux?
- What specific Claude Code hooks are available/useful for this?
- Should the auto-setup be a shell script (like lspmux-rust-analyzer) or a Rust CLI?
- How to handle workspace detection - does lspmux auto-detect the workspace root, or does it need explicit configuration?
- Version management: pin rust-analyzer version or auto-update?

## References

- [lspmux crate](https://docs.rs/lspmux/0.3.0/lspmux/) - Core LSP multiplexer
- [lspx](https://jsr.io/@frontside/lspx) - JS LSP multiplexer (reference for merge strategies)
- [lspmux-rust-analyzer](https://github.com/sunshowers/lspmux-rust-analyzer) - Setup kit we're wrapping
