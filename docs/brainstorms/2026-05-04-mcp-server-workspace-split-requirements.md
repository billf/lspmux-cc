---
date: 2026-05-04
topic: mcp-server-testability
status: narrowed
---

# MCP server testability boundary

**Status (2026-09-04):** The proposed four-crate workspace is superseded. It
was based on a premise that is no longer true: `mcp-server` is already one
Cargo package with a `lspmux_cc_mcp` library target, and its integration test
imports `LspClient` from that library. Do not split the package merely to make
code testable.

## What remains

`bootstrap`, `lsp_client`, and `telemetry` are public modules of the existing
library. The remaining boundary is `tools.rs`, which is still a private module
of the binary. That prevents external tests from exercising the same tool
parameter validation and response shaping that the MCP server dispatches.

The smallest useful follow-up is therefore:

1. move or expose `tools` through the existing library target;
2. make `main.rs` consume that library module rather than a duplicate private
   module; and
3. add only the hermetic tests that require the public boundary.

This retains one package, one dependency graph, and the existing binary name.
It does not need a Cargo workspace, new crate names, `publish = false` guards,
or Nix/Justfile rewiring.

## Test contract

The follow-up should prove behavior rather than crate topology:

- invalid or non-absolute file paths become MCP invalid-parameter errors and
  record the matching telemetry outcome;
- response shaping preserves the documented zero-based input and one-based
  location-output convention, including null and empty LSP responses; and
- a fake LSP transport can exercise tool dispatch without a live
  rust-analyzer, where that is not already adequately covered by unit tests.

Keep the existing integration test for two clients sharing rust-analyzer as a
separate end-to-end invariant. Bootstrap and LSP-client behavior are already
importable for focused tests; add a regression test there only for a concrete
bug, not as a prerequisite to exposing `tools`.

## Scope boundaries

- No four-crate split (`client`, `tools`, `telemetry`, and binary).
- No API-stability or publishing commitment; the library remains
  workspace-internal.
- No MCP tool additions, changed argument shapes, or telemetry redesign.
- No fabricated service process or rust-analyzer in ordinary unit tests.
- No Nix, plugin, or shell-wrapper change unless the small module move proves
  one is necessary.

## Why the original split was retired

The original proposal correctly identified that a binary-private `tools.rs`
limits integration tests. Its remedy was disproportionate: the repository had
already extracted the LSP client, bootstrap, and telemetry into the existing
library, so three of the four proposed crates duplicated boundaries that
already exist. Splitting them would add public cross-crate visibility, package
metadata, and build rewiring without increasing the relevant test surface.

ARCH-1 and ARCH-3 remain useful historical findings, but they no longer justify
the four-crate design. Treat this note as their narrowed disposition. Any
implementation plan that still treats the full workspace split as a prerequisite
should be updated to this boundary.

## References

- [`mcp-server/src/lib.rs`](../../mcp-server/src/lib.rs) — current library
  boundary
- [`mcp-server/tests/integration.rs`](../../mcp-server/tests/integration.rs) —
  existing external library consumer
- ARCH-1 and ARCH-3 historical review records (the retired tracker tree is not
  part of this repository)
