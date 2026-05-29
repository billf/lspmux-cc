---
title: "fix: Drop rust-analyzer override from overlays.default"
status: active
date: 2026-05-28
type: fix
origin: https://github.com/billf/lspmux-cc/issues/2
---

# fix: Drop `rust-analyzer` override from `overlays.default`

## Summary

`overlays.default` exports a `rust-analyzer` attribute that replaces nixpkgs' top-level `rust-analyzer` with the fenix nightly build. That replacement changes the hash of `rusty-v8-rust-toolchain` (which bundles `rust-analyzer`), cascading into `rusty-v8` and `deno` and forcing a ~40-minute source rebuild on every flake bump for downstream consumers on `aarch64-darwin`. The fix drops the `rust-analyzer` attribute from the overlay while keeping the additive `rust-analyzer-nightly`. Internal consumers (the `plugin` package, the devShell) keep working because they reference `packages.rust-analyzer`, not the overlay.

---

## Problem Frame

`flake.nix` uses flake-parts `easyOverlay`, which turns `overlayAttrs` into `overlays.default`. Today (`flake.nix:52-54`):

```nix
overlayAttrs = {
  inherit (config.packages) lspmux-cc-mcp lspmux rust-analyzer rust-analyzer-nightly plugin;
};
```

The `rust-analyzer` entry shadows nixpkgs' top-level `rust-analyzer`. Any downstream overlay that splats `inputs.lspmux-cc.overlays.default` inherits that shadow, and the hash cascade documented in the issue follows:

`rust-analyzer` → `rusty-v8-rust-toolchain` → `rusty-v8-147.4.0` → `deno-2.7.14`, all losing `cache.nixos.org` coverage.

`rust-analyzer-nightly` is an alias of the same fenix derivation (`flake.nix:33-34`: `rust-analyzer-nightly = rust-analyzer`) and is additive — it does not exist in nixpkgs, so exporting it causes no cache miss. Consumers that want nightly opt in explicitly via `pkgs.rust-analyzer-nightly`.

---

## Requirements

- **R1.** `overlays.default` no longer exports an attribute named `rust-analyzer`. After the change, `builtins.attrNames (overlays.default {} {})` returns `["lspmux", "lspmux-cc-mcp", "plugin", "rust-analyzer-nightly"]` (no `rust-analyzer`).
- **R2.** `overlays.default` continues to export `rust-analyzer-nightly` (additive opt-in for nightly).
- **R3.** Internal consumers stay intact: `nix build .#plugin` and `nix develop` still resolve a rust-analyzer binary (via `packages.rust-analyzer`, which is internal, not the overlay).
- **R4.** `nix flake check` continues to pass (clippy, fmt, tests, `plugin-structure`).
- **R5.** The breaking change is recorded for downstream consumers (release note / commit body), since any consumer relying on `pkgs.rust-analyzer` resolving to nightly after applying this overlay must migrate to `pkgs.rust-analyzer-nightly`.

---

## Key Technical Decisions

**KTD1 — Drop the overlay attribute, keep the package.** Remove `rust-analyzer` from `overlayAttrs` only. Leave `packages.rust-analyzer` (`flake.nix:69`) in place: the `plugin` derivation (`flake.nix:86`, `self'.packages.rust-analyzer`) and the devShell (`flake.nix:133`, `self'.packages.rust-analyzer`) both reference it internally. Packages are not part of the consumer overlay surface, so keeping them does not reintroduce the cascade.

*Rationale:* The cascade is caused exclusively by the overlay *replacing* a nixpkgs top-level attr. `config.packages.rust-analyzer` is namespaced under this flake's own package set and never collides with a consumer's nixpkgs. The issue's own diagnostic confirms the overlay attr is the load-bearing one.

**KTD2 — Do not rename `packages.rust-analyzer`.** Tempting to rename it to `rust-analyzer-nightly`-only for clarity, but that's churn beyond the issue's scope and would touch the plugin store-path symlink name (`$out/bin/rust-analyzer`) and `plugin-structure` check assertions. Keep the internal name; the fix is surgical.

**KTD3 — Preserve `overlayAttrs` ordering minus the dropped attr.** The remaining inherit list reads `lspmux-cc-mcp lspmux rust-analyzer-nightly plugin` — the same set minus `rust-analyzer`.

---

## Implementation Units

### U1. Drop `rust-analyzer` from `overlayAttrs`

**Goal:** Stop the overlay from replacing nixpkgs' top-level `rust-analyzer`.

**Requirements:** R1, R2, R3.

**Dependencies:** none.

**Files:**
- `flake.nix` (modify `overlayAttrs`, lines 52-54)

**Approach:** Remove the `rust-analyzer` token from the `inherit (config.packages) ...` list inside `overlayAttrs`. Resulting block:

```nix
overlayAttrs = {
  inherit (config.packages) lspmux-cc-mcp lspmux rust-analyzer-nightly plugin;
};
```

Leave `flake.nix:33-34` (the `let`-bound `rust-analyzer` / `rust-analyzer-nightly` alias), `flake.nix:69` (`packages` `inherit rust-analyzer rust-analyzer-nightly`), `flake.nix:86` (plugin symlink), and `flake.nix:133` (devShell) untouched.

**Patterns to follow:** The existing flake-parts `easyOverlay` pattern — `overlayAttrs` is the single source for `overlays.default`. No new pattern introduced.

**Test scenarios:**
- Covers R1. Evaluate `nix eval --impure --expr 'builtins.attrNames ((builtins.getFlake (toString ./.)).overlays.default {} {})'` (or the current-system equivalent) and assert the result list does **not** contain `"rust-analyzer"` and **does** contain `"rust-analyzer-nightly"`, `"lspmux"`, `"lspmux-cc-mcp"`, `"plugin"`.
- Covers R3. Run `nix build .#plugin` and assert it succeeds and `result/bin/rust-analyzer` resolves to an executable (the plugin still wires its own rust-analyzer via the internal package).
- Covers R4. Run `nix flake check` and assert all checks (`clippy`, `fmt`, `tests`, `plugin-structure`) pass.

**Verification:** `overlays.default` attribute names no longer include `rust-analyzer`; `nix build .#plugin` and `nix flake check` both succeed.

### U2. Record the breaking change for downstream consumers

**Goal:** Make R5 durable — consumers relying on `pkgs.rust-analyzer` (post-overlay) learn they must switch to `pkgs.rust-analyzer-nightly`.

**Requirements:** R5.

**Dependencies:** U1.

**Files:**
- Commit message body (primary durable record), and the PR description.
- `README.md` — only if a consumer-facing overlay-usage section is added; today the README documents `RUST_ANALYZER_PATH` and a PATH-based binary, not a `pkgs.rust-analyzer` overlay contract, so no existing section requires editing. Add a short "Overlay" note documenting `rust-analyzer-nightly` as the opt-in attribute **only if** it improves consumer clarity.

**Approach:** Capture the breaking-change note in the commit body and PR description: the overlay no longer shadows `pkgs.rust-analyzer`; consumers wanting nightly use `lib.getExe pkgs.rust-analyzer-nightly`. Reference issue #2.

**Test expectation:** none — documentation/metadata only.

**Verification:** Commit body and PR description state the breaking change and the migration path.

---

## Scope Boundaries

In scope:
- Removing the `rust-analyzer` attribute from `overlays.default`.
- Recording the breaking change.

### Deferred to Follow-Up Work
- Renaming the internal `packages.rust-analyzer` / plugin symlink for naming consistency (KTD2). Not required to fix the cascade.

Out of scope:
- Changing which rust-analyzer build the plugin or devShell uses (they keep using the fenix build internally).
- Any change to `rust-analyzer-nightly` itself.

---

## Risks & Dependencies

- **Breaking change for downstream consumers.** Any consumer relying on `pkgs.rust-analyzer` resolving to nightly *after* applying this overlay loses that resolution. Per the issue, that set is probably just this flake's own examples; mitigated by R5 (release note + migration path to `pkgs.rust-analyzer-nightly`). The downstream that filed the issue already works around it via `removeAttrs`, so this change removes the need for that workaround.
- **No internal regression risk.** The `plugin` package and devShell reference `packages.rust-analyzer` (internal namespace), not the overlay, so they are unaffected — verified by R3/R4.

---

## Verification

End-to-end, from the repo root:

1. **Overlay surface (R1, R2):**
   ```bash
   nix eval --impure --expr \
     'builtins.attrNames ((builtins.getFlake (toString ./.)).overlays.default {} {})'
   ```
   Expect a list with `rust-analyzer-nightly` present and `rust-analyzer` absent.

2. **Internal consumers (R3):**
   ```bash
   nix build .#plugin && test -x "$(readlink result/bin/rust-analyzer)"
   ```

3. **Full checks (R4):**
   ```bash
   nix flake check
   ```
   All of `clippy`, `fmt`, `tests`, `plugin-structure` pass.

4. **Downstream sanity (optional, R5):** A consumer splatting `inputs.lspmux-cc.overlays.default` no longer needs the `removeAttrs ... ["rust-analyzer"]` workaround and no longer triggers a `rusty-v8` / `deno` source rebuild on flake bump.

---

## Sources & Research

- Issue: https://github.com/billf/lspmux-cc/issues/2
- Full diagnostic write-up (downstream): `billf/dotfiles` `docs/solutions/build-errors/lspmux-overlay-rust-analyzer-replacement-rebuilds-deno-2026-05-26.md`
- Downstream workaround commit: `billf/dotfiles@17aebf9`
- flake-parts `easyOverlay` module: turns `overlayAttrs` into `overlays.default`.
