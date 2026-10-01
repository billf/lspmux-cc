# Migrating to the on-demand spawn model (M5)

Before M5, `./setup core` installed a launchd plist (macOS) or systemd user unit (Linux) that auto-started `lspmux server` at login. The MCP server assumed the daemon was already running.

After M5, the MCP server spawns `lspmux server` itself when it first needs the daemon. No system service manager is involved. The legacy plist/unit, if still installed, wastes RAM and creates a second daemon competing for the same socket.

## Migrating

```
./setup migrate
```

This unloads `com.lspmux.server` (macOS) or disables `lspmux.service` (Linux), then removes the file. Safe to run when nothing is installed — it reports "Nothing to migrate."

Verify:

```
launchctl print gui/$(id -u)/com.lspmux.server   # macOS: "Could not find service"
systemctl --user is-active lspmux.service        # Linux: "inactive"
```

## Detection from inside an MCP session

`rust_server_status` returns a `legacy_global_daemon_detected: true` flag when a service-manager unit is still loaded. `./setup doctor` surfaces the same signal with a `[~~]` marker and points at `./setup migrate`.

## Deferred managed-daemon mode

A managed daemon is not supported today. `auto` only reuses a reachable daemon
or directly spawns one; it never registers, starts, or owns a launchd/systemd
unit. The simpler model avoids competing daemons and makes ordinary bootstrap
portable.

Reconsider a managed mode only when users need a durable daemon lifecycle that
on-demand spawning cannot provide. It must be an explicit bootstrap mode with
explicit unit installation and lifecycle commands, version-aware upgrades,
standard log locations, and integration coverage for install, upgrade, reuse,
and removal. A service manager must never be an implicit side effect of `auto`.

## Nix-darwin / home-manager users

If your Nix configuration declares the plist (e.g. `launchd.user.agents.lspmux`), `./setup migrate` will remove the deployed file, but Nix will reinstall it on the next system rebuild. Remove the declaration from your Nix config too.

## Why this change

The post-M1 integration test `two_worktrees_get_separate_rust_analyzers` proved that upstream lspmux already multiplexes multiple workspaces through one socket — there is no per-workspace correctness benefit to the auto-start model that the launchd plist provided. On-demand spawn removes a moving part (the service manager) and a class of "stale daemon left over from yesterday's lspmux version" bugs.
