---
title: "Harden external binary contracts and simplify bootstrap"
status: completed
date: 2026-09-08
type: maintenance
---

# Harden external binary contracts and simplify bootstrap

## Delivered

- Bootstrap validates `lspmux --version` before reuse or direct spawn and
  supports only `>=0.3.0, <0.4.0`; errors state the detected state and the
  `lspmux server`-without-`--config` contract.
- `./setup core` pins new installations to 0.3.0 and validates existing
  executables. Rust-analyzer remains externally selected, with best-effort
  path/version logging documented in [binary-resolution.md](../binary-resolution.md).
- `auto` is reuse-or-direct-spawn only. Manager startup and
  `LSPMUX_ALLOW_MANAGER_BOOTSTRAP` are removed; passive legacy-unit detection
  and `./setup migrate` remain.

## Boundaries

REV-011 continues to own endpoint-scoped spawn leasing and daemon file logging.
This change does not reimplement those mechanisms.

The review suggestion in [PR #7 discussion
3521022244](https://github.com/billf/lspmux-cc/pull/7#discussion_r3521022244)
belongs with the tool-annotation change it reviews: once those annotations land,
their regression test should enumerate `RustAnalyzerTools::tool_router().list_all()`
rather than a hand-maintained tool array. It is intentionally not applied to
this bootstrap-focused change because the annotated tool set is not on this
branch.
