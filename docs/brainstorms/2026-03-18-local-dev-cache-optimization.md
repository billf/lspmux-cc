# Local dev cache optimization: sccache, multi-RA, and artifact sharing

**Date:** 2026-03-18
**Status:** Brainstorm
**Context:** Follow-up to `docs/brainstorms/2026-03-18-observability-and-reuse-roadmap.md` and `todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md` (REV-005)

## The goal

If I open a file in two editors, run `cargo test` from one editor, and run `cargo clippy` against the same package, the artifacts should be mostly reused. A single shared rust-analyzer per worktree is necessary but not sufficient. The compilation cache layer is the second half.

## sccache Nix sandbox problem

`mcp-server/target/.rustc_info.json` contains:

```
"stderr":"sccache: error: Operation not permitted (os error 1)\n"
```

sccache was injected (likely via `RUSTC_WRAPPER` in the Nix dev shell) but can't run inside Nix's sandbox due to macOS sandbox restrictions. `.direnv/flake-profile` sets `CARGO_BUILD_INCREMENTAL=false` but doesn't explicitly disable sccache.

**Impact:** `nix build` and `nix flake check` run without sccache (they use Nix's own caching). `just check/clippy/test` run outside the sandbox but inherit the broken sccache wrapper, so every compilation invocation pays a sccache startup + failure overhead before falling back to direct rustc.

**Fix options:**
1. Explicitly set `RUSTC_WRAPPER=""` in the Nix dev shell to disable sccache for local cargo commands
2. Configure sccache to work outside the Nix sandbox (set `SCCACHE_DIR` to a user-writable path)
3. Use `RUSTC_WRAPPER=sccache` only in `.cargo/config.toml` (which Nix builds ignore)

## Artifact sharing across cargo commands

When rust-analyzer triggers a check build and the user then runs `cargo clippy`, the clippy run can reuse check artifacts if:
- Same compiler version
- Same feature flags
- Same profile (`dev` vs `release`)
- Same `target/` directory

The current setup has one `target/` directory per workspace. cargo's fingerprint system handles reuse automatically within a single `target/`. The problem arises when:
- `nix flake check` builds in an isolated derivation (separate `target/`, no reuse from local dev)
- Multiple worktrees have separate `target/` dirs (no cross-worktree reuse)
- sccache is broken (no cross-invocation cache)

## Multi-RA cache sharing

If multiple rust-analyzer instances (one per worktree, per the per-worktree brainstorm) each trigger cargo builds, can they share a compilation cache?

**rust-analyzer's `cargo.targetDir` setting** controls where RA writes build artifacts. If two RA instances point at the same `targetDir`, they'd share artifacts. But cargo's file locking would serialize their builds, potentially causing the exact lock contention this project is trying to avoid.

**sccache is the better path for cross-RA reuse.** Each RA writes to its own `target/`, but sccache deduplicates at the rustc invocation level. Two identical `rustc` invocations on different worktrees of the same commit would both hit sccache. Different commits would partially hit (unchanged crates still match).

## sccache observability

`sccache --show-stats --stats-format json` provides structured stats:
- `compile_requests`: total compilations seen
- `cache_hits.counts`: hits per language (Rust, C, etc.)
- `cache_misses.counts`: misses per language
- `cache_size`: current cache size in bytes
- `max_cache_size`: configured limit
- `cache_write_duration`, `cache_read_hit_duration`: timing stats

Stats deltas can be captured by snapshotting before/after build windows:

```bash
BEFORE="$(sccache --show-stats --stats-format json 2>/dev/null)"
# ... build window ...
AFTER="$(sccache --show-stats --stats-format json 2>/dev/null)"
# diff cache_hits.counts and cache_misses.counts
```

From Rust: `tokio::process::Command::new("sccache").args(["--show-stats", "--stats-format", "json"])` and deserialize the JSON.

This connects to REV-005 (compiler action accounting): REV-005 measures *whether* artifacts were reused. This brainstorm addresses *how* to make reuse more likely.

## Design options

### Option A: Fix sccache for local dev

1. Set `RUSTC_WRAPPER=sccache` in `.cargo/config.toml` (cargo-level, not Nix-level)
2. Set `SCCACHE_DIR` to a user-writable path outside the Nix store
3. Unset `RUSTC_WRAPPER` in the Nix dev shell to avoid conflicts
4. Both `just` commands and rust-analyzer use the same sccache instance

### Option B: Shared target directory (per-repo, not per-worktree)

Use `rust-analyzer.cargo.targetDir` to point all RA instances for the same repo at a single `target/` directory. cargo's file locking serializes builds. Works for reuse but reintroduces the lock contention problem.

### Option C: Nix-cached builds for CI, sccache for local dev

Accept that `nix flake check` and `just cargo` use different cache strategies. Nix uses `cargoArtifacts` (already implemented). Local dev uses sccache. The two don't share, but each is internally consistent.

### Option D: cargo-nextest for parallel test execution

Unrelated to caching but relevant to the "run cargo test" scenario. `cargo-nextest` can run tests in parallel across packages. Doesn't help with cache sharing but reduces wall-clock time for the same amount of compilation.

## What this repository is responsible for

This repository owns the shared rust-analyzer per worktree. Compilation caching (sccache) and build coordination (cargo lock contention) are adjacent but largely outside its scope.

The intersection: if this repository's bootstrap process or config templates can set up sccache correctly for the RA environment (via `pass_environment` in lspmux.toml or wrapper scripts), that's a natural place to do it. Recording sccache stats deltas around RA-triggered builds (REV-005) is also in scope.

Everything else (sccache server lifecycle, cross-project cache sharing, CI cache warming) is infrastructure work that happens outside this repo.

## Related

- `todos/2026-03-18-compiler-action-and-artifact-reuse-accounting.md` (REV-005) — measuring reuse
- `docs/brainstorms/2026-03-18-per-worktree-socket-routing.md` — per-worktree RA instances
- `docs/brainstorms/2026-03-18-observability-and-reuse-roadmap.md` — overall roadmap
- `config/lspmux.toml` — `pass_environment` already passes `CARGO_HOME`
