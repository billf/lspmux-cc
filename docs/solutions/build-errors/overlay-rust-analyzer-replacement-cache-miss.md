---
title: "Overlay replacing top-level rust-analyzer cascades into rusty-v8 / deno cache misses"
status: resolved
date: 2026-05-28
category: build-errors
module: "Nix flake overlay (overlays.default)"
problem_type: build_error
component: tooling
symptoms:
  - "~40-minute source rebuild of rusty-v8 and deno on aarch64-darwin on every flake bump"
  - "pkgs.deno and pkgs.rusty-v8 diverge from cache.nixos.org at the same flake-pinned nixpkgs rev"
  - "Only appears in downstream flakes that splat inputs.lspmux-cc.overlays.default into their overlay"
root_cause: config_error
resolution_type: config_change
severity: high
tags: [nix, overlay, rust-analyzer, fenix, deno, flake-parts, cache-miss, easyoverlay]
---

# Overlay replacing top-level rust-analyzer cascades into rusty-v8 / deno cache misses

## Problem

Before this fix, `overlays.default` exported a `rust-analyzer` attribute that **replaced** nixpkgs' top-level `rust-analyzer` with the fenix nightly build. A downstream flake applying the overlay could therefore miss binary-cache substitutions for `rusty-v8` and `deno` and rebuild them from source on `aarch64-darwin` after a flake update. The incident took roughly 40 minutes per rebuild.

## Symptoms

- Downstream `nix build` / `darwin-rebuild` compiles `rusty-v8` and `deno` from source instead of substituting the matching nixpkgs outputs.
- `pkgs.deno.outPath` differs from the upstream nixpkgs `deno.outPath` at the *same* pinned nixpkgs rev.
- The divergence only shows up after applying `inputs.lspmux-cc.overlays.default`.

## What Didn't Work

- Treating it as a stale-cache or substituter problem. The store paths legitimately differ; no amount of cache configuration helps because the derivation hash genuinely changed.
- A downstream workaround stripped the attribute at the consumer (`builtins.removeAttrs (inputs.lspmux-cc.overlays.default final prev) ["rust-analyzer"]`). This works but every consumer has to know to do it — the fix belongs upstream in the overlay.

## Solution

Drop the `rust-analyzer` attribute from `overlayAttrs` (this flake uses flake-parts `easyOverlay`, which turns `overlayAttrs` into `overlays.default`). Keep the additive `rust-analyzer-nightly`. This shipped in commit `3e161cd`.

```nix
# flake.nix — before
overlayAttrs = {
  inherit (config.packages) lspmux-cc-mcp lspmux rust-analyzer rust-analyzer-nightly plugin;
};

# flake.nix — after
overlayAttrs = {
  # rust-analyzer is intentionally NOT exported: it shadows nixpkgs' top-level
  # rust-analyzer and cascades into rusty-v8 -> deno rebuilds. rust-analyzer-nightly
  # is additive (no nixpkgs collision); consumers opt in via it explicitly.
  inherit (config.packages) lspmux-cc-mcp lspmux rust-analyzer-nightly plugin;
};
```

The internal `packages.rust-analyzer` stays — the `plugin` derivation symlink and the devShell reference `self'.packages.rust-analyzer`, which is namespaced under this flake and never collides with a consumer's nixpkgs.

Downstream consumers who want nightly opt in explicitly:

```nix
{ rustAnalyzerPath = lib.getExe pkgs.rust-analyzer-nightly; }
```

## Why This Works

At the nixpkgs revision involved in the incident, Deno's `rusty-v8-rust-toolchain` resolved the top-level `rust-analyzer` package. Replacing that package changed the toolchain's input hash, which cascaded:

`rust-analyzer` → `rusty-v8-rust-toolchain` → `rusty-v8` → `deno`

Those changed derivations no longer matched the binary-cache outputs for that nixpkgs revision, so they rebuilt from source. The exact Deno and rusty-v8 versions are intentionally omitted: they change as nixpkgs advances. `rust-analyzer-nightly` is the *same* fenix derivation but is exported under a distinct name, so it is additive: nixpkgs does not resolve its normal `rust-analyzer` dependency through that attribute, avoiding the hash change and cascade.

## Prevention

- **Never export an attribute from an overlay whose name collides with a nixpkgs top-level package unless replacement is the explicit intent.** Overlays splat into the consumer's `pkgs`; a colliding name silently rewrites their package set and every downstream derivation that depends on it. Prefer a distinct, additive name (`<tool>-nightly`, `<tool>-pinned`).
- **Keep build-internal packages out of the consumer overlay surface.** `config.packages.<x>` is namespaced to the flake; `overlayAttrs.<x>` is not. Only promote to the overlay what consumers genuinely need.
- **Verify the overlay's exported attribute set** when changing `overlayAttrs`:
  ```bash
  nix eval --impure --json --expr \
    'let f = builtins.getFlake (toString ./.); p = import f.inputs.nixpkgs { system = builtins.currentSystem; }; in builtins.attrNames (f.overlays.default p p)'
  # expect: ["lspmux","lspmux-cc-mcp","plugin","rust-analyzer-nightly"]  (no "rust-analyzer")
  ```
  Do not probe with `overlays.default {} {}`: easyOverlay needs a real package set to determine `hostPlatform`. Applying it to nixpkgs as above also tests the path consumers use. A `nix flake check` assertion on this attr set would lock the intent in against regression (not yet added).

## Related Issues

- Issue #2 (this repo); fixed by commit `3e161cd`.
- Full root-cause write-up lives in the downstream consumer repo: `billf/dotfiles` `docs/solutions/build-errors/lspmux-overlay-rust-analyzer-replacement-rebuilds-deno-2026-05-26.md`; downstream workaround commit `billf/dotfiles@17aebf9`.
- Plan: `docs/plans/2026-05-28-001-fix-overlay-rust-analyzer-replacement-plan.md`.
