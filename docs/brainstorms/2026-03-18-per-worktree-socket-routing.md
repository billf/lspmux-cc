# Per-worktree socket routing for multi-workspace scenarios

**Date:** 2026-03-18
**Status:** Brainstorm
**Context:** Follow-up to `docs/brainstorms/2026-02-05-lspmux-claude-code-brainstorm.md` and `docs/brainstorms/2026-03-18-observability-and-reuse-roadmap.md`

## The problem

The current architecture uses a single global Unix socket (`$XDG_RUNTIME_DIR/lspmux/lspmux.sock`). If a user opens two git worktrees of the same repo, both sessions connect to the same lspmux instance with the first session's workspace root. rust-analyzer produces incorrect results for the second worktree.

Zero references to "worktree" exist in the runtime code. `default_socket_path()` in `bootstrap.rs` produces one path regardless of `WORKSPACE_ROOT`.

## Why one RA can't serve two worktrees

The LSP protocol binds `rootUri` at initialization and doesn't support changing it mid-session. VS Code, Neovim, and every major editor start a separate language server per workspace folder. The structural reasons:

- Different file contents on different branches
- Different `Cargo.lock` resolutions
- Different `target/` directories and proc-macro outputs
- `rootUri` is singular, set once at init

`linkedProjects` adds non-overlapping projects, not two copies of the same repo. `workspace.discoverConfig` is for non-Cargo build systems (Buck2, Bazel).

**One lspmux per worktree is the correct model.**

## What lspmux upstream provides (and doesn't)

Neither lspmux nor lspx has multi-workspace routing. lspmux's model: one config, one socket, one LSP server per instance. Multi-worktree support must live in lspmux-cc's orchestration layer.

lspmux supports both Unix socket and TCP via `listen`/`connect` config. `instance_timeout = 300` shuts down idle instances automatically. Each instance reads one config file.

## Proposed socket naming

**Constraint:** `sun_path` is 104 bytes on macOS, 108 on Linux. Full workspace paths blow past this.

**Scheme:**

```
$RUNTIME_BASE/lspmux/<sha256-first-12-hex>/lspmux.sock
```

12 hex chars (48 bits) gives negligible collision probability across hundreds of worktrees. Example: `/tmp/lspmux/a1b2c3d4e5f6/lspmux.sock` (~42 bytes, well under limit).

**Registry file** at `$RUNTIME_BASE/lspmux/registry.json` maps hashes to worktree roots, PIDs, and creation timestamps. This keeps socket paths short while maintaining debuggability.

Precedent: Nix store paths use hash-of-inputs. SSH agent uses `$TMPDIR/ssh-XXXX/agent.<pid>`. Docker uses hash-based container IDs.

## Worktree detection

The reliable command: `git -C "$dir" rev-parse --show-toplevel`. Returns the correct worktree root for both main and linked worktrees. Two worktrees of the same repo return different paths.

Canonicalize with `std::fs::canonicalize` (Rust) or `realpath` (shell) before hashing to prevent symlink-aliased paths from producing different hashes.

Edge cases: non-git directories (fall back to `$WORKSPACE_ROOT` or `pwd`, which the code already does), bare repos with worktrees (still works), detached worktrees (still works).

## Service model

**Don't use persistent launchd/systemd units for per-worktree instances.** launchd lacks systemd's template units (`service@instance`). Even on Linux, persistent units per worktree create cleanup headaches (stale units when worktrees are deleted, `KeepAlive` restarting processes for gone worktrees).

**On-demand spawn with idle timeout.** The first client needing a worktree spawns its lspmux instance directly (the `start_direct_server` path already exists in `bootstrap.rs`). lspmux's `instance_timeout = 300` handles cleanup automatically. This is the pattern sccache and rust-analyzer itself use.

Keep launchd/systemd only for a single "primary" workspace if you want auto-start-on-login behavior. On Linux, template units work for opt-in: `lspmux@<hash>.service`.

## Race prevention

Two editors opening the same worktree simultaneously: use a filesystem lock (`flock` on `$socket_dir/spawn.lock`) before spawning. After acquiring the lock, re-check the socket. If it appeared, release and connect. In Rust, use the `fs2` crate or `nix::fcntl::flock`.

## Stale socket handling

Before connecting, attempt a non-blocking connect or run `lspmux status` against the per-worktree socket. If `ECONNREFUSED`, unlink the stale socket and spawn fresh. Optionally maintain a PID file for faster liveness checks (`kill -0 $pid`).

## Abandoned worktree cleanup

The 5-minute idle timeout is the primary cleanup mechanism. Socket dirs on macOS `$TMPDIR` are cleaned on reboot. For thoroughness, a lazy GC step during session-start: walk registry, remove entries whose worktree roots no longer exist on disk.

## Key architectural change

`socket_path` becomes a *derived* value from `workspace_root` rather than a separate env var. `RuntimeConfig` already has `workspace_root`; the change is making `socket_path` a function of it.

**Implementation impact:**

- `mcp-server/src/bootstrap.rs` — replace single `socket_path` with per-worktree socket resolution (worktree detection, hash computation, config generation, spawn locking)
- `setup` — generate a template config or remove config-writing entirely (let the MCP server generate configs on demand)
- `plugins/lspmux-rust-cc/hooks/scripts/session-start.sh` — pass worktree-specific socket path when checking/starting service
- `config/lspmux.toml` — template becomes per-instance, generated at runtime
- `launchd/com.lspmux.server.plist` — keep only for primary workspace; per-worktree instances spawn on demand

## Open questions

- Should `LSPMUX_SOCKET_PATH` env var override per-worktree derivation? (Probably yes, for backwards compatibility.)
- What happens if a user renames a worktree directory? The hash changes, orphaning the old socket. The idle timeout cleans it up, but there's a window.
- Should the registry file be human-editable or strictly machine-managed?
- Can the per-worktree config be generated in-memory and passed via stdin instead of writing a file?

## Related

- `todos/2026-03-18-observability-and-client-attribution.md` (REV-004) — client/workspace attribution
- `docs/brainstorms/2026-03-18-observability-and-reuse-roadmap.md` — "one RA per worktree" as a stated goal
- `docs/brainstorms/2026-02-05-lspmux-claude-code-brainstorm.md` — original architecture
