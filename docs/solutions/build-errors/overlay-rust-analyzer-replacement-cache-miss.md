---
title: "Overlay replacing top-level rust-analyzer cascades into rusty-v8 / deno cache misses"
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

`overlays.default` exported a `rust-analyzer` attribute that **replaced** nixpkgs' top-level `rust-analyzer` with the fenix nightly build. Any downstream flake that splatted the overlay inherited a ~40-minute source rebuild of `rusty-v8` and `deno` on `aarch64-darwin`, on every flake bump.

## Symptoms

- Downstream `nix build` / `darwin-rebuild` triggers a long source compile of `rusty-v8-147.4.0` and `deno-2.7.14` that should have been a cache hit.
- `pkgs.deno.outPath` differs from the upstream nixpkgs `deno.outPath` at the *same* pinned nixpkgs rev.
- The divergence only shows up after applying `inputs.lspmux-cc.overlays.default`.

## What Didn't Work

- Treating it as a stale-cache or substituter problem. The store paths legitimately differ; no amount of cache configuration helps because the derivation hash genuinely changed.
- A downstream workaround stripped the attribute at the consumer (`builtins.removeAttrs (inputs.lspmux-cc.overlays.default final prev) ["rust-analyzer"]`). This works but every consumer has to know to do it — the fix belongs upstream in the overlay.

## Solution

Drop the `rust-analyzer` attribute from `overlayAttrs` (this flake uses flake-parts `easyOverlay`, which turns `overlayAttrs` into `overlays.default`). Keep the additive `rust-analyzer-nightly`.

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

Shipped in PR #4 (merged); fixes issue #2.

## Why This Works

nixpkgs bundles every Rust tool — rustc, cargo, clippy, rustfmt, **and rust-analyzer** — into one `rusty-v8-rust-toolchain` derivation. Overriding the top-level `rust-analyzer` changes that derivation's input hash, which cascades:

`rust-analyzer` → `rusty-v8-rust-toolchain` → `rusty-v8-147.4.0` → `deno-2.7.14`

All three lose `cache.nixos.org` coverage and rebuild from source. `rust-analyzer-nightly` is the *same* fenix derivation but exported under a name nixpkgs doesn't use, so it's purely additive — nothing in nixpkgs depends on an attribute called `rust-analyzer-nightly`, so no hash changes and no cascade.

## Prevention

- **Never export an attribute from an overlay whose name collides with a nixpkgs top-level package unless replacement is the explicit intent.** Overlays splat into the consumer's `pkgs`; a colliding name silently rewrites their package set and every downstream derivation that depends on it. Prefer a distinct, additive name (`<tool>-nightly`, `<tool>-pinned`).
- **Keep build-internal packages out of the consumer overlay surface.** `config.packages.<x>` is namespaced to the flake; `overlayAttrs.<x>` is not. Only promote to the overlay what consumers genuinely need.
- **Verify the overlay's exported attribute set** when changing `overlayAttrs`:
  ```bash
  nix eval --impure --expr \
    'let f = builtins.getFlake (toString ./.); p = import f.inputs.nixpkgs { system = builtins.currentSystem; }; in builtins.attrNames (f.overlays.default p p)'
  # expect: ["lspmux","lspmux-cc-mcp","plugin","rust-analyzer-nightly"]  (no "rust-analyzer")
  ```
  Note the `overlays.default {} {}` probe from the original diagnostic no longer works with easyOverlay's eta-expansion (it can't determine `hostPlatform` from empty sets) — apply the overlay to a real nixpkgs as above. A `nix flake check` assertion on this attr set would lock the intent in against regression (not yet added).

## Related Issues

- Issue #2 (this repo) and PR #4 (the fix).
- Full root-cause write-up lives in the downstream consumer repo: `billf/dotfiles` `docs/solutions/build-errors/lspmux-overlay-rust-analyzer-replacement-rebuilds-deno-2026-05-26.md`; downstream workaround commit `billf/dotfiles@17aebf9`.
- Plan: `docs/plans/2026-05-28-001-fix-overlay-rust-analyzer-replacement-plan.md`.
