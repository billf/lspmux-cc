---
date: 2026-05-04
topic: mcp-server-workspace-split
---

# `mcp-server` Workspace Split for Testability

> **Note (2026-05-28):** this split subsumes two findings from the archived
> 2026-03-18 review: ARCH-1 (`tools.rs` trapped in the binary crate, blocking
> integration tests) and ARCH-3 (`lsp_client.rs` over-scoped, extract `uri.rs`
> and `language.rs`). Both are addressed by the crate breakdown below and are not
> tracked separately. See `todos/archive/review-2026-03-18-p1-critical.md` and
> `todos/archive/review-2026-03-18-p2-important.md`.

## Summary

Split the single-package `mcp-server/` crate into a 4-crate Cargo workspace —
`lspmux-cc-client` (LSP-over-lspmux client + bootstrap), `lspmux-cc-tools` (MCP
tool implementations), `lspmux-cc-telemetry` (telemetry types), and the
existing `lspmux-cc-mcp` binary — to make LSP client state, bootstrap
behavior, tool parameter validation, and tool response shaping
integration-testable.

---

## Problem Frame

ARCH-1, named in the prior ideation pass: `mcp-server/src/tools.rs` is ~700
lines of LSP-protocol logic, plus `lsp_client.rs`, plus the bootstrap fixes
that landed in April 2026 (`LSPMUX_CONFIG_PATH` propagation, TCP-vs-Unix-socket
detection, explicit `LSPMUX_CONNECT` override). All of it lives in a single
binary crate. Integration tests cannot import any of it because there is no
library target — the only test path today is unit tests inside the binary,
which can't exercise tool dispatch, LSP state tracking, or response shaping
end-to-end.

The recent commit history is exactly the shape of code that needs a test
harness: small fixes to runtime-coupled behavior with no automated regression
guard. The next bootstrap or transport bug will ship the same way.

A separate concern surfaced during the doc-review of the ideation pass: the
ideation doc justified the split with platform-ambitions framing ("alternate
client interfaces, polyglot LSP expansion, downstream reuse"), which is
speculative scope inflation. This brainstorm scopes the work to testability
plus **two acknowledged non-testability architectural calls**: (a) telemetry
as its own crate (chosen for naming-by-content; details in Key Decisions), and
(b) bootstrap absorbed into `lspmux-cc-client` (forced by the dependency
graph — `tools` imports `RuntimeStatus`/`SERVER_NAME` from bootstrap, so
bootstrap-in-binary doesn't compile). Neither call is challenged here. The
platform framing remains excluded.

---

## Requirements

**Crate layout**

- R1. The workspace root is `mcp-server/Cargo.toml` with `[workspace]` declaring
  members for the four crates listed in R2-R5. The current single `[package]`
  is removed; the binary crate becomes a workspace member.
- R2. `lspmux-cc-mcp` (binary) — rmcp transport setup, `main`, argument
  parsing, and the startup choreography that today lives in `main.rs`:
  `RuntimeConfig::discover()`, `ensure_service_running()`, `LspClient::new()`,
  and `TelemetryState::from_env()`. Populates `ClientIdentity` from env vars
  at startup. Direct dependencies on **all three** library crates:
  `lspmux-cc-client` (uses `RuntimeConfig`, `SERVER_NAME`, `ServiceMode`,
  `LspClient`), `lspmux-cc-tools` (uses `RustAnalyzerTools`), and
  `lspmux-cc-telemetry` (uses `TelemetryState`, reads bootstrap-result
  enums). The earlier draft listed only tools+telemetry; that was wrong —
  `main.rs:14-16` imports from all three surfaces directly.
- R3. `lspmux-cc-client` (library) — the LSP-over-lspmux client (subprocess
  spawn of `lspmux client`, LSP JSON-RPC framing, pending-request tracking,
  document state with `ensure_file_open` content-hash dedup, `didOpen`/
  `didChange` ordering), plus the bootstrap module (mode resolution, TCP-vs-
  Unix detection, `LSPMUX_CONFIG_PATH` propagation, `LSPMUX_CONNECT` override)
  including the `RuntimeStatus` and `SERVER_NAME` types that today live in
  `bootstrap.rs`. Public exports: the `LspClient` API (`request`,
  `notification`, `ensure_file_open`, etc.), the `RuntimeConfig` /
  `RuntimeStatus` types, and the `SERVER_NAME` constant. Depends on
  `lspmux-cc-telemetry`. **Caveat**: this crate now bundles two distinct
  content domains — LSP protocol (lsp_client) and lspmux-cc-specific service
  discovery (bootstrap). Naming-by-content would put bootstrap in its own
  crate; the dependency-graph constraint trumped naming this round. If
  contributors find this a real cognitive cost, the future-round
  bootstrap-extraction candidate is the relief valve.
- R4. `lspmux-cc-tools` (library) — MCP tool implementations on top of the
  client: parameter validation (zero/one-based handling, absolute-path
  requirement on `file_path`), LSP request dispatch, and LSP-to-MCP response
  shaping (hover markdown, goto-definition `Location` to `file:line:col`,
  workspace-symbol formatting, server-status snapshot construction). Depends on
  `lspmux-cc-client` (uses both the `LspClient` API for request dispatch and
  the bootstrap types `RuntimeStatus`/`SERVER_NAME` for server-status response
  construction) and `lspmux-cc-telemetry` (records `ToolTelemetry`, reads
  `TelemetrySnapshot` for `server_status`).
- R5. `lspmux-cc-telemetry` (library) — `ClientIdentity`,
  `BootstrapTelemetry`, `ToolTelemetry`, `CompilerAccountingSnapshot`,
  `TelemetrySnapshot`, `ReadinessState`, `ToolOutcome`, and the
  `TelemetryState` container with `record_*` and `snapshot` methods. The
  helper functions `now_unix_ms` and `system_time_ms` are imported by client
  (see Dependencies). No dependencies on the other three crates. Producers
  (binary for `ClientIdentity`; client for bootstrap outcomes and
  `ReadinessState`; tools for `ToolOutcome` writes) and consumers (tools'
  `server_status` reads `TelemetrySnapshot`/`CompilerAccountingSnapshot`/
  `ReadinessState`) all import this crate. **`ReadinessState` and
  `ToolOutcome` must be `pub` on the crate root** — both are used across
  crate boundaries (`lsp_client.rs:29` reads `ReadinessState`; `tools.rs:27`
  reads both). Earlier draft of R5 omitted them; the split would not
  compile without them exported.
- R6. Workspace layout under `mcp-server/` defaults to flat —
  `mcp-server/{mcp,client,tools,telemetry}/`. The existing `mcp-server/src/`
  tree dissolves; modules move into per-crate `src/` directories under each
  member. The `crates/` subdir variant is left to planning.

**Test surface**

- R7a. The split must enable integration tests for **LSP client state
  tracking** — `ensure_file_open` content-hash dedup correctness,
  pending-request tracking under concurrent calls, `didOpen`/`didChange`
  ordering, response routing for out-of-order LSP messages. Harness shape: a
  fake `lspmux client` subprocess or in-process LSP server stub that speaks
  LSP framing; tests do not require a live `rust-analyzer` instance.
- R7b. The split must enable integration tests for **bootstrap behavior** —
  mode resolution (`Auto`/`Require`/`Off` with `LSPMUX_BOOTSTRAP` env),
  TCP-vs-Unix-socket detection, `LSPMUX_CONNECT` override handling,
  `validate_prerequisites()` enforcement. Harness shape is **distinct** from
  R7a: bootstrap probes `lspmux server` (not the client) and runs before the
  LSP child, so the fake `lspmux client` subprocess does not exercise it.
  Bootstrap tests use `TcpListener::bind`/`UnixListener::bind` for the
  `Reused` path, tempdir-backed `lspmux_path`/`server_path`/`config_path` for
  `validate_prerequisites`, and a small fake `lspmux server` binary built as
  a dev-dependency for the `StartedDirectly` branch.
- R8. The split must enable integration tests for tool parameter validation
  as it currently exists — zero-based line/character inputs handled via
  `PositionParam`, absolute-path requirement on `file_path` (`tools.rs`
  `validate_absolute_path`), `WorkspaceSymbolParam::query: String` forwarded
  unmodified to `lsp.workspace_symbols`, and error mapping from invalid
  input (missing/non-absolute path, malformed JSON params) to MCP error
  responses with the correct `ToolOutcome::InvalidParams` telemetry write.
  No new validation behavior is introduced; the earlier draft mentioned
  "range validation on `workspace_symbol` queries" — there is no such
  validation today, and adding one would break the no-new-behavior scope
  boundary.
- R9. The split must enable integration tests for tool response shaping — LSP
  response transformation to MCP tool response (hover markdown, goto-
  definition file:line:col one-based output), null/empty response handling.
- R10. New tests can land in the same commit as the move, in a follow-up
  commit, or interleaved with module moves. There is no constraint on PR
  shape or commit ordering.

**API stability and documentation**

- R11. Each library crate has a top-of-`lib.rs` doc comment that names the
  crate as workspace-internal and disclaims API stability. Suggested wording:
  `//! Internal to the lspmux-cc workspace. No API stability guarantees;
  consumers outside this workspace are not supported.`
- R12. The four crates are not published to crates.io. Cargo.toml entries
  include `publish = false` to enforce this at the build-system level.

---

## Success Criteria

- **Falsifiable on day one (proxy regressions reproduced as tests that fail
  when the fix is reverted).** The new harness reproduces three specific
  regression scenarios as integration tests on day one, with each test
  covering a distinct testability surface:
  - (a) **LSP-client (R7a):** `ensure_file_open` correctly suppresses a
    redundant `didChange` notification when the file content hash is
    unchanged across calls. Reverting the content-hash dedup check makes the
    test fail because a duplicate `didChange` is observed by the fake LSP
    server stub.
  - (b) **Bootstrap (R7b):** `service_ready` correctly detects a
    pre-bound TCP listener at `127.0.0.1:<port>` and returns `Reused` from
    `ensure_service_running` rather than spawning a duplicate. Reverting the
    TCP-detection branch in `service_ready` makes the test fail because
    `start_direct_server` is invoked when it shouldn't be.
  - (c) **Bootstrap (R7b):** when both the lspmux config file's `connect`
    field and the `LSPMUX_CONNECT` env var are set,
    `RuntimeConfig::discover` uses the **config-file** value (current
    behavior at `bootstrap.rs:179-182`: `fs::read_to_string(config_path)
    .and_then(parse_connect_addr).or_else(|| connect_hint...)`). Reverting
    the precedence (env-first via `connect_hint.or_else(parse_connect_addr
    on file)`) makes the test fail because `connect_addr` resolves to the
    env value rather than the file value. Note: the earlier proxy
    description claimed env-overrides-config; that contradicts the
    implementation. This is the load-bearing precedence the harness must
    lock in. If a future change actually wants env-overrides-config, that
    is an intentional behavior fix and belongs out of pure-refactor scope.
  Each test must have a documented "fail when fix reverted" expectation
  recorded in its source comment so future maintainers can validate the
  proxy is load-bearing rather than a smoke test. The earlier
  `LSPMUX_CONFIG_PATH` proxy was dropped because that fix lives in plugin
  glue (`.mcp.json`), not in `mcp-server` Rust — it's not reproducible by
  any harness inside the workspace.

  **Definition of done.** The split is complete when **all three**
  conditions hold simultaneously: (1) crate boundaries compile (`cargo
  check`, `nix flake check`); (2) the three day-zero proxy tests above each
  fail when their corresponding fix is reverted, then pass when restored;
  (3) `cargo`/`nix`/plugin behavior is unchanged from pre-split (the No
  regression bullet below). R10 leaves test landing flexible relative to
  module moves, but the doc is not "complete" until the day-zero contract
  is met.
- **Forward-looking outcome.** The next runtime-coupled fix in the bootstrap
  or LSP client area lands with at least one integration test that would have
  caught the bug. The "regression magnet" framing from ARCH-1 stops applying.
- **Handoff outcome.** `ce-plan` can produce a migration plan without
  re-deriving the crate layout, telemetry placement, dependency direction, or
  test surface from ideation context.
- **No regression — the split is a pure refactor by external observation.**
  `cargo check`/`build`/`clippy --all-targets -- -W clippy::nursery -W
  clippy::pedantic`/`fmt --all`/`test`, `nix flake check`, `nix build`, and the
  plugin's MCP behavior in Claude Code are all observably unchanged. The
  `lspmux-cc-mcp` binary lands at the same path inside the plugin derivation
  with the same behavior.

---

## Scope Boundaries

- Publishing any of the four crates to crates.io.
- Changing the existing MCP tool surface (no new tools, no removed tools, no
  argument-shape changes).
- Restructuring telemetry recording so the binary intercepts every tool
  dispatch (Key Decisions option C). That is a behavior change, not a
  refactor.
- Polyglot LSP expansion or a language-agnostic LSP fabric.
- Alternate client interfaces (CLI, HTTP, gRPC) on top of the new libraries.
- Restructuring `bin/` shell wrappers' triple-tier binary resolution.
- New behaviors in `tools.rs`, `lsp_client.rs`, or `bootstrap.rs`. Modules
  move; contents are preserved verbatim where possible (modulo `pub`
  visibility changes required by the new crate boundaries).
- PR-level reviewability constraints — single-author project; commit
  granularity is whatever's convenient. **Temporal assumption**: this stance
  expires the moment a contributor lands a PR. If that happens, atomic-commit
  treatment for workspace-shape changes resumes. The decision is not
  permanent.

---

## Key Decisions

- **Decision: bootstrap moves into `lspmux-cc-client`.** Rationale:
  `mcp-server/src/tools.rs:24` imports `RuntimeStatus` and `SERVER_NAME` from
  the bootstrap module, and `RustAnalyzerTools` carries a
  `runtime: RuntimeStatus` field. With bootstrap in the binary and tools as a
  library, tools cannot compile without circular-depending on the binary or
  extracting the bootstrap types into a separate crate. Moving bootstrap into
  client resolves the dependency cleanly (tools imports through client) AND
  brings bootstrap behavior into the testable surface (R7), closing the
  warrant-vs-exclusion contradiction the doc-review flagged. An earlier draft
  of this doc kept bootstrap in the binary; that draft did not survive
  feasibility review.
- **Decision: split into four crates, with telemetry standalone — one of
  two non-testability architectural calls.** Three of four crates (binary,
  client, tools) are testability-driven and scope-aligned. Telemetry as its
  own crate is one architectural call that goes beyond strict testability
  scope. Promoted on naming-by-content grounds: telemetry has multiple
  producers — the **binary** (writes `ClientIdentity` at startup), client
  (writes bootstrap outcomes), tools (writes tool outcomes) — and at least
  one consumer (tools' `server_status` reads snapshots). The
  multi-**producer** asymmetry is load-bearing: the binary is one of the
  producers and cannot live inside any of the library crates, so putting
  telemetry types inside any single library crate would force the binary
  to depend on that crate purely for telemetry writes. A standalone telemetry
  crate avoids this. The doc-review flagged this overall split as
  architecture-aesthetics-as-strategy; this brainstorm acknowledges the
  judgment honestly rather than re-justifying it as testability. Cost: one
  extra `Cargo.toml`. Acceptable. Revisit triggers captured below.
- **Decision logic captured for future revisit (telemetry placement).**
  Three options were considered and the trade-offs recorded so a future
  round can re-evaluate without re-deriving them:
  - **A. Types in `lspmux-cc-client`.** Rejected. The load-bearing argument
    is that the binary is also a telemetry producer (writes `ClientIdentity`
    at startup). Putting types in client would force the binary to depend
    on client purely for telemetry writes, even though the binary doesn't
    use the LSP-client API directly (it goes through tools for that). A
    standalone telemetry crate keeps the binary's dependency on telemetry
    explicit and orthogonal to its dependency on tools. Earlier draft of
    this rejection cited "client is already the publishing surface for
    everything" — that framing was wrong: bootstrap telemetry production
    *is* in client under the selected design too, so option A and option B
    are identical on producer concentration. The real argument is binary's
    role as a producer, not client's.
  - **B. Standalone `lspmux-cc-telemetry` crate.** Selected. Names content
    directly per the project's naming-by-content principle. Producer and
    consumer crates depend on it without depending on each other for
    telemetry.
  - **C. Restructure: binary intercepts every tool dispatch and records
    there. Tools become pure transformations.** Deferred. Cleanest
    separation, but moves the recording site (a behavior change, not a
    refactor). If C eventually happens, B's multi-producer rationale weakens
    and a 3-crate collapse becomes the natural follow-up. See Future-round
    candidates.
- **Decision: crate names follow naming-by-content; "core" is rejected.**
  Rationale: "core" is a content-free container name. Each crate here is
  named for what is inside it: client (the LSP client + bootstrap + the
  types it owns), tools (MCP tool implementations), telemetry (telemetry
  container). The binary keeps `lspmux-cc-mcp`.
- **Decision: APIs are unstable, explicitly documented as such (R11
  retained).** Rationale: workspace-internal consumers only today. The
  doc-review flagged R11 as speculative against non-existent external
  consumers; user retained the requirement on the basis that documenting
  the no-stability stance at the top of each `lib.rs` is cheaper than
  versioning and sets correct expectations if someone outside the workspace
  does stumble in. Reversible per crate when an external consumer
  materializes.
- **Decision: `publish = false` in each library Cargo.toml (R12 retained).**
  Rationale: Scope Boundaries already excludes publishing, but R12 enforces
  it at the build-system level so a future contributor can't accidentally
  publish via a release pipeline change. The doc-review flagged this as
  defense against an excluded action; user retained the guard.
- **Decision: scope is testability plus two non-testability architectural
  calls.** Rationale: the testability case (bootstrap, client, tools all
  become testable) stands on its own warrant. Two non-testability calls are
  baked in honestly: (a) telemetry as its own crate (chosen for
  naming-by-content + binary-as-producer asymmetry); (b) bootstrap absorbed
  into client (forced by the dependency graph — `tools` imports
  `RuntimeStatus`/`SERVER_NAME` from bootstrap). Both are acknowledged
  rather than re-justified as testability. The platform-ambitions framing
  from the prior ideation pass remains excluded.

---

## Dependencies / Assumptions

- The existing `mcp-server/Cargo.toml` is currently a single `[package]`,
  not a `[workspace]`. (Verified during the prior doc-review feasibility
  pass — this contradicted a claim in the ideation doc, which has since
  been corrected.) The split introduces a workspace; it does not extend
  one.
- The Justfile pins `--manifest-path mcp-server/Cargo.toml` for `cargo`
  invocations. Workspace root at `mcp-server/Cargo.toml` keeps the manifest
  path stable. Recipe-level adjustments may be needed (e.g., `--workspace`
  for test-all-crates), but no path changes.
- The Nix flake uses crane to build the existing crate. After the split,
  `flake.nix` needs `pname = "lspmux-cc-mcp"` (or
  `cargoExtraArgs = "-p lspmux-cc-mcp"`) on `craneLib.buildPackage`;
  `cargoArtifacts` rebuilt for the workspace; `cargoTest`/`cargoClippy`/
  `cargoFmt` invocations may need `--workspace`. The plugin derivation
  continues to ship `lspmux-cc-mcp` at the same store path because the
  binary's name is preserved.
- `now_unix_ms` and `system_time_ms` in `telemetry.rs` are currently
  `pub(crate)`. After the split, `lspmux-cc-client` (which imports both for
  bootstrap and `lsp_client.rs` use) cannot see `pub(crate)` items from
  `lspmux-cc-telemetry`. Either both helpers become `pub` on the telemetry
  crate, or they duplicate into the consumer. Mechanical decision deferred
  to implementation.
- The existing `mcp-server/tests/integration.rs` imports
  `lspmux_cc_mcp::lsp_client::LspClient`. After the split, the import
  becomes `lspmux_cc_client::LspClient`, and the test file moves into
  `lspmux-cc-client/tests/`. Mechanical.
- No external consumers of `lspmux-cc-client`, `lspmux-cc-tools`, or
  `lspmux-cc-telemetry` exist today. The libs become workspace-internal.
- The April 2026 bootstrap fixes (`LSPMUX_CONFIG_PATH` propagation, TCP
  detection, `LSPMUX_CONNECT` override) move with the bootstrap module
  into `lspmux-cc-client`. Their behavior is preserved verbatim; this work
  does not modify them. Note: the `LSPMUX_CONFIG_PATH` propagation fix
  itself lived in plugin glue (`.mcp.json` env injection by the Claude
  Code MCP host), not in the Rust code being moved. Inside the binary
  `RuntimeConfig::discover()` is a one-line `std::env::var` read; that
  behavior is not regression-prone in a way the Rust harness can guard.
  This is why the Success Criteria proxy bug list dropped
  `LSPMUX_CONFIG_PATH` and added an LSP-client `ensure_file_open` proxy
  instead.
- **Bootstrap-test harness setup cost.** R7b's bootstrap tests require
  on-disk prerequisites because `RuntimeConfig::ensure_service_running()`
  calls `validate_prerequisites()`, which checks `Path::new(...).exists()`
  for `lspmux_path`, `server_path`, and `config_path`. Tempdir placeholders
  satisfy this for `Reused` paths (where a pre-bound TCP listener
  short-circuits the spawn) and `Off` mode. Testing `StartedDirectly`
  needs a real fake `lspmux server` binary — built as a workspace
  dev-dependency. Planning needs to pick the harness composition before
  R7b can claim full coverage of bootstrap modes.

---

## Outstanding Questions

### Resolve Before Planning

- *(none — all product-shape decisions are captured above.)*

### Deferred to Planning

- **[Affects R6][Technical]** Flat layout
  (`mcp-server/{mcp,client,tools,telemetry}/`) versus `crates/` subdir
  (`mcp-server/crates/{mcp,client,tools,telemetry}/`). Default flat;
  planning may pick `crates/` if it reads cleaner with four members or if
  a fifth crate (e.g., bootstrap-extraction follow-up) becomes likely.
- **[Affects R3, R4, R5][Technical]** Module-internal visibility cleanup —
  some `pub(crate)` items will need to become `pub` to cross crate
  boundaries (notably `now_unix_ms`/`system_time_ms` per Dependencies);
  some today-`pub` items can stay `pub(crate)` inside their new crate.
  Mechanical; resolved by `cargo check` passes after the move.
- **[Affects R7a][Needs research]** Fake `lspmux client` subprocess shape
  for the LSP-client harness (R7a) — Rust binary built as a test fixture,
  shell script, or in-process LSP server stub (`tower-lsp-server` or
  similar). Each has trade-offs for fidelity vs. setup cost.
- **[Affects R7b][Needs research]** Fake `lspmux server` binary for the
  bootstrap harness (R7b's `StartedDirectly` branch) — minimum behavior is
  to `bind` the requested socket/port and stay connectable. `service_ready`
  / `wait_for_socket` only call `tcp_is_ready` / `socket_is_ready`
  (`bootstrap.rs:270-287`); no LSP `Initialize` is sent during bootstrap, so
  the fake does not need to speak LSP framing. Built as a workspace
  dev-dependency.
- **[Affects R7a/R7b][Acceptance]** The harnesses must each demonstrate
  the day-zero proxy contract: the LSP-client test for
  `ensure_file_open` dedup must fail when the dedup check is reverted; the
  bootstrap tests for TCP detection and `LSPMUX_CONNECT` override must
  fail when their respective fixes are reverted. If a chosen harness
  shape cannot demonstrate fix-revert failure, escalate before committing.
- **[Affects all R][Technical]** Workspace introduction + module moves +
  Nix flake rewires + Justfile recipe updates — one commit or several? No
  PR-reviewability constraint here (single-author), so this is a
  preference question resolved at implementation time.

### Future-round candidates (revisit when prioritized)

Each candidate carries a concrete trigger condition. "We'll figure it out
later" without a named trigger has been actively avoided.

- **Restructure telemetry recording so the binary intercepts every tool
  dispatch (Key Decisions option C).** Pure separation win — tools become
  transformations that don't know they're measured — but it is a behavior
  change, not a refactor. **Trigger:** any of (a) telemetry gains a fourth
  producer crate, (b) `tools` needs to be tested without a `telemetry`
  dependency, (c) the recording-site coupling makes it impossible to add a
  tool without also touching telemetry. If implemented, B's multi-producer
  rationale for telemetry-as-its-own-crate weakens, and a 3-crate collapse
  (telemetry folds into client) becomes the natural follow-up.
- **Extract bootstrap into its own crate (`lspmux-cc-bootstrap`).** Today
  bootstrap rides inside `lspmux-cc-client`, which is a dependency-graph
  expedient acknowledged in R3 as bundling two distinct content domains.
  **Trigger:** any of (a) a contributor reports the
  LSP-protocol-vs-service-discovery mix in `lspmux-cc-client` as a real
  cognitive cost, (b) bootstrap acquires a fourth concern beyond its
  current four (mode resolution, transport detection, config-path
  propagation, connect-override) such that it grows past `lsp_client.rs` in
  size, (c) any test in `lspmux-cc-tools` finds the transitive dependency
  on launchd/systemd plumbing through `lspmux-cc-client` actively painful.
- **Reconsider crate boundaries if a real consumer of `lspmux-cc-tools` or
  `lspmux-cc-telemetry` emerges outside the workspace.** Note: a real
  external consumer of `lspmux-cc-client` is structurally unlikely because
  bootstrap couples it to lspmux-cc's specific service-discovery model;
  unbundling bootstrap (the candidate above) is the prerequisite for this
  trigger to fire on `lspmux-cc-client` specifically.
