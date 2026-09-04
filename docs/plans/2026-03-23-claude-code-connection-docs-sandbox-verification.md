---
title: "Claude Code connection, sandbox, and verification"
status: superseded
created: 2026-03-23
superseded: 2026-09-04
type: documentation
---

# Claude Code connection, sandbox, and verification

## Disposition

Superseded. The original plan assumed a launchd-managed Unix-socket daemon and
therefore prescribed `LSPMUX_BOOTSTRAP=require`. The current integration uses
on-demand bootstrap (`LSPMUX_BOOTSTRAP=auto`) and supports TCP loopback, which
is the preferred sandbox-compatible transport. Retaining the old plan as an
implementation checklist would now direct users toward a less reliable setup.

The current, authoritative guidance is:

- [Claude Code host guide](../hosts/claude-code.md) for installation,
  transport selection, sandbox setup, and verification.
- `./setup sandbox claude-code` for exact Unix-socket allowlisting when Unix
  sockets are needed.
- `./setup doctor` and `rust_server_status` for diagnostics.

## What remains valid

- macOS sandboxing can block Unix-domain socket connections; allowlist the
  exact socket path rather than enabling all Unix sockets.
- TCP loopback avoids that allowlist but is appropriate only for a local,
  single-user machine because lspmux does not authenticate that endpoint.
- The MCP server should return a descriptive tool error during degraded startup
  rather than turning an expected retryable condition into a protocol failure.

## Why it was superseded

The plan's proposed plugin-root paths, mandatory pre-started service,
`LSPMUX_BOOTSTRAP=require`, fixed socket examples, and several Claude CLI/lifecycle
assertions no longer match the repository's supported configuration. The host
guide is maintained with the runtime contract; this historical note deliberately
does not duplicate it.
