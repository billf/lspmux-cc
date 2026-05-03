---
date: 2026-04-30
topic: lspmux-cc-open-ideation
focus: surprise-me (no user-specified subject)
mode: repo-grounded
---

# Ideation: lspmux-cc — Surprise-Me Pass

## Context

This is an open-ended ideation pass on lspmux-cc — a Claude Code plugin that
multiplexes editor LSP clients (Neovim, Claude Code, Codex) through a Unix socket
or 127.0.0.1:27631 TCP to a shared rust-analyzer via upstream lspmux. The user
asked the agent to pick the focus ("surprise me"). The artifact lives here in the
plan file because plan mode is active; on `ExitPlanMode` approval, the user can
pick one idea to take into `/ce-brainstorm`, optionally save the file to
`docs/ideation/`, or refine in conversation.

## Grounding

**Project shape.** Rust + shell. mcp-server/ exposes 6 read-only MCP tools
(diagnostics, hover, goto-definition, find-references, workspace-symbol,
server-status) over rmcp v0.15. Distributed as a Claude Code plugin via Nix flake.
macOS launchd LaunchAgent. v0.2.0. Currently the **only** RA-MCP project sharing
a persistent rust-analyzer instance — the differentiator is concrete (RA cold
start: seconds to minutes; 1GB+ RAM baseline; 40GB pathological).

**Named pain points (from codebase scan).** REV-009 hooks suppress errors with
`2>/dev/null || true`. REV-008 no response cache, no in-flight coalescing.
ARCH-1 tools.rs (702 LOC) trapped in binary crate, untestable. AGENT-1/2 no
`rust_code_actions` or `rust_rename`. REV-006 launchd/systemd parity drift.
Sandbox `allowUnixSockets` friction. Non-Nix install path is ~30 minutes.

**External signal.** 3 competing RA-MCP servers (zeenix, Benedikt Terhechte's
RAT — 19 tools, isaacphi/mcp-language-server). None share a persistent daemon.
LSP spec issue #1160: multi-client deferred to backlog at protocol level.
lsp-devtools records LSP traffic to SQLite (lspmux-cc has no traffic log). MCP
2026 roadmap moves sessions to data layer; lazy schema loading via ToolSearch
removes the wide-tool-surface objection. nREPL session model, Bazel persistent
workers, tmux/mosh, Watchman, PgBouncer all map structurally. LLM Rust failures
dominated by hallucinated APIs — pre-generation hover query short-circuits the
write→diagnose→fix loop.

**Recent activity (last ~6 weeks).** Flattened plugin to repo root. MCP bootstrap
fixes (LSPMUX_CONFIG_PATH, TCP detection, LSPMUX_CONNECT override). Diagnostic
skill repo-agnostic. Hook fail-fast on broken LSPMUX_PATH.

## Ranked Ideas

### 1. Add `rust_code_actions` and `rust_rename` MCP tools

**Description:** Expose `textDocument/codeAction` and `textDocument/rename` from
rust-analyzer through MCP. Today an agent reads a diagnostic, hallucinates a fix,
edits the file textually, and re-runs diagnostics in a loop. With code actions,
the agent receives RA's canonical fix (auto-import, derive trait, fix field name)
and applies it precisely. Decide once whether the agent applies the WorkspaceEdit
or the server does.

**Warrant:** `direct:` AGENT-1/2 named in grounding ("no rust_code_actions, no
rust_rename — agents guess fixes and edit manually"). `external:` Benedikt
Terhechte's RAT exposes 19 tools — the bar for what an RA-MCP server offers
has moved.

**Rationale:** The whole point of plugging an LSP into an LLM is tool-grounded
answers instead of probabilistic guesses. Code actions are the highest-signal
LSP capability lspmux-cc currently leaves on the table. MCP 2026 lazy schema
loading via ToolSearch removed the schema-bloat objection.

**Downsides:** Edit application semantics are subtle (workspace-wide rename
touches files the agent isn't editing). Read→write flips the trust boundary;
needs a feature flag at minimum. Additional surface area for bugs.

**Confidence:** 85%
**Complexity:** Medium
**Status:** Unexplored

---

### 2. Promote `tools.rs` to an `lspmux-cc-core` library crate

**Description:** Split `mcp-server/` into a workspace with `lspmux-cc-core`
(LSP client, tool implementations, telemetry, bootstrap) and a thin
`lspmux-cc-mcp` binary that wires rmcp to the core. Integration tests target
the library; the binary stays a transport adapter.

**Warrant:** `direct:` ARCH-1 named in grounding ("tools.rs (702 LOC) trapped
in binary crate, can't be integration-tested"). Cargo workspace structure
already exists under `mcp-server/` — adding a sibling crate is mechanical.

**Rationale:** 702 LOC of LSP-protocol logic with no integration tests is a
regression magnet. The April 2026 bootstrap fixes (TCP detection,
LSPMUX_CONFIG_PATH propagation) are exactly the code that needs a test harness
and is currently impossible to test. Library boundary also unlocks alternate
frontends (CLI, HTTP, gRPC), polyglot LSP expansion, and reuse by the 3
competing RA-MCP servers as a daemon-sharing layer.

**Downsides:** Cargo workspace touch-ups. Versioning and API stability decisions
if the core crate is published. None block the move; all are normal library work.

**Confidence:** 95%
**Complexity:** Low
**Status:** Unexplored

> Note: this is the prerequisite for ideas #1, #4, and a future polyglot pivot.
> If you do one thing first, this is the leverage move.

---

### 3. First-class workspace-roaming sessions

**Description:** Promote session handles from connection-lifecycle implicit to
first-class data. Session key = `(client_kind, cwd, RUST_SRC_PATH, toolchain)`.
At MCP handshake, detect mismatches and either reinitialize, reject with a clear
error, or attach to an existing session. Enables reattach across Claude Code
restarts, multi-attach (human + agent on the same session), and session-scoped
caching for idea #4.

**Warrant:** `external:` grounding's mosh analogy is direct ("lspmux has tmux
half but not the mosh half — what happens when Claude Code reopens with
different cwd or RUST_SRC_PATH"). nREPL's session-as-first-class-concept and
MCP 2026's roadmap of moving sessions into the data model layer both validate.

**Rationale:** Today's design has a latent landmine: cd between two Rust
workspaces and diagnostics go silently wrong. Sessions also unblock
multi-tenant scoping, capability tokens, replay/undo, and the
"agent + human pair-programming on the same RA state" workflow.

**Downsides:** Significant protocol surface change. Requires deciding
session-scoped vs daemon-scoped state for every existing piece of state
(open documents, watched files, diagnostics subscriptions). PgBouncer's
transaction-mode-vs-session-mode design tax is real here.

**Confidence:** 75%
**Complexity:** Medium-High
**Status:** Unexplored

---

### 4. Push diagnostic subscriptions instead of pull

**Description:** Add a subscription channel where MCP clients register interest
in diagnostics for a file glob; the daemon fans out RA's
`textDocument/publishDiagnostics` notifications to subscribers (with a
ring-buffer backlog so reconnecting clients catch up). Replaces the current
pattern where every tool call is poll: agent edits, agent forgets to re-check,
diagnostics drift.

**Warrant:** `external:` Watchman's subscription model is the canonical example.
RA already emits `publishDiagnostics`; the daemon currently flattens it to
request-response. p2502/lspmux's documented "server-to-client requests are
dropped" gap (LSP issue #1160) is exactly what subscriptions close.

**Rationale:** Long-running agent loops (ralph, ultrawork) need to react to
"you just introduced a type error" within a turn, not on the next manual query.
Push beats poll for tight feedback loops, and REV-008 (cache/coalescing
pressure) drops out of the design when push replaces poll.

**Downsides:** MCP transport must support server-initiated messages (notifications
or resources). State management complexity — subscribers come and go, backlogs
need bounds. Timing-sensitive bugs become possible.

**Confidence:** 70%
**Complexity:** Medium-High
**Status:** Unexplored

---

### 5. Per-toolchain × per-worktree multiplex axis

**Description:** Today's mental model: one rust-analyzer per workspace root.
Reframe: pool RA instances by `(toolchain, sysroot, RUST_SRC_PATH)` tuple, with
overlays for sibling git worktrees that share a base. Two crates on the same
nightly share one RA + one std index. Stacked-branch worktrees (git-spice
workflow) share the dep graph; only the user's edits diverge.

**Warrant:** `reasoned:` cold-start cost (1GB+ RAM, 30s indexing) is dominated
by toolchain/sysroot indexing, not workspace files. Two workspaces on the same
nightly should share; one workspace switching channels should not. `direct:`
user uses `git-spice` per global CLAUDE.md memory and TCP `127.0.0.1:27631`
is the configured local transport — both signal stacked-branch + multi-RA
workflows are real here.

**Rationale:** Stacked-branch workflows create N near-identical worktrees that
today each get their own RA + 1GB RAM + multi-second cold start. This is
exactly the developer pattern that punishes lspmux-cc's current design hardest,
and it's a pattern the user actively uses. RA's salsa engine can in principle
overlay edits on a shared base; even without that, sharing by toolchain alone
is a clean win.

**Downsides:** Worktree overlay is the harder half — RA upstream may not support
shared-base + per-client overlay cleanly. Toolchain-keyed pooling alone delivers
most of the value and is tractable.

**Confidence:** 70%
**Complexity:** Medium (toolchain-keyed) / High (worktree overlay)
**Status:** Unexplored

---

### 6. Distribution + onboarding triad: Homebrew tap, auto-allowlist socket, JSON doctor

**Description:** Three coordinated pieces tackling the install cliff.
**(a) Homebrew tap:** generate `brew tap liquidlabs/lspmux-cc` from the existing
flake outputs. Static binary + plist + post-install doctor. Nix stays as truth;
brew is the door.
**(b) Auto-allowlist:** on first successful socket bind, merge the path into
`~/.claude/settings.json` `allowUnixSockets` with a `lspmux-cc:managed` marker
for idempotency. The most insidious onboarding cliff (working daemon, broken
plugin, no error) closes.
**(c) JSON doctor:** `bin/lspmux-cc-doctor` emits a single structured JSON
document covering daemon state, transport reachability, launchd plist status,
recent hook errors, and rust-analyzer version match. Bug reports paste it; CI
asserts against it; the diagnose-lspmux skill consumes it.

**Warrant:** `direct:` grounding lists "Sandbox: socket must be explicitly listed
in ~/.claude/settings.json allowUnixSockets — silent connect failure otherwise"
plus "Brew + pre-built binary + launchctl load would cover Homebrew-native devs"
and "No competing RA-MCP server uses Nix". REV-009 (hook errors invisible) is
the symptom doctor cures. `external:` Watchman's `watchman-diag` and Bazel's
`bazel info` show the JSON-diag pattern is the right shape.

**Rationale:** The shared-daemon differentiator is real but unreachable — the
30-minute Nix onboarding is a wall for non-Nix devs and the silent
allowUnixSockets failure is a wall for everyone. Three small wins compound: a
Homebrew dev gets `brew install` → daemon starts → socket auto-allowlisted →
doctor reports green → Claude Code sees diagnostics. Today none of those steps
work without manual intervention.

**Downsides:** Three streams of work. Brew tap requires a release pipeline
(GitHub Actions cross-compile, signing). Auto-allowlist edits a user-owned
config file — needs careful idempotency and an opt-out. Doctor adds maintenance
surface (every new failure mode = one more probe).

**Confidence:** 80%
**Complexity:** Medium (each piece is Low-Medium; coordination is the cost)
**Status:** Unexplored

---

### 7. SQLite traffic recorder, opt-out

**Description:** Always-on rolling SQLite recorder (size-bounded, e.g. 100MB
ring) capturing every JSON-RPC frame between MCP server and lspmux client.
Diagnosis becomes "open the DB" instead of "reproduce with logging enabled."
Bug reports become `lspmux-cc-doctor --replay bug.db`. Every REV-009-class
hook error gets captured once and replayed forever.

**Warrant:** `external:` lsp-devtools (swyddfa) proven pattern — SQLite-backed
recorder + TUI inspection. Grounding explicitly notes "lspmux-cc has no
traffic log capability." `direct:` REV-009 (silent hook failure) and REV-008
(redundant requests) are exactly the bugs a traffic log surfaces in seconds.

**Rationale:** docs/solutions/ is empty. There's no compounding artifact yet.
Trace files become that artifact: they accrue, they're shareable, they're
diff-able, and they make the project legible to outsiders without reading Rust.
The hardest lspmux-cc bugs will be timing- and ordering-sensitive between three
editors and one rust-analyzer; a recorder makes them reproducible test fixtures.

**Downsides:** Privacy — Rust source content can leak into diagnostic payloads;
needs redaction or an opt-out switch (rather than opt-in). Disk usage
management. Schema design is a one-shot decision that's painful to evolve.

**Confidence:** 75%
**Complexity:** Medium
**Status:** Unexplored

---

## Cross-Cutting Notes

- **#2 (library crate) is the prerequisite leverage move.** Doing #2 first makes
  #1 (code actions), #4 (subscriptions), and any future polyglot pivot strictly
  cheaper.
- **#3 (sessions) and #4 (subscriptions) compose.** Subscriptions are
  session-scoped or daemon-scoped, and the answer changes the design of both.
  Worth thinking about together even if implemented separately.
- **#6 (distribution triad) is the only survivor that monetizes the existing
  differentiator.** The shared-daemon win is concrete but unreachable today
  for non-Nix users.
- **#7 (traffic recorder) compounds with #6's doctor.** The doctor produces
  a structured snapshot; the recorder produces a structured stream. Together
  they replace "describe what you saw" with "paste this DB."

## Rejection Summary

| # | Idea | Reason Rejected |
|---|------|-----------------|
| 1 | Replace `ensure_file_open` content-hash dedup with RA didOpen/didChange source-of-truth | Risky for unclear win; loses a working optimization |
| 2 | Delete `bin/` triple-tier resolution entirely | Recent activity shows it's still being tuned; resolution exists for documented reasons |
| 3 | Remove `server-status` tool, move to MCP handshake | Below meeting-test floor — small refactor, doesn't warrant team discussion |
| 4 | Auto-derive `.lsp.json` from Cargo.toml | `.lsp.json` is platform format, not freely changeable; impractical |
| 5 | Kill `LSPMUX_CONNECT` override, auto-probe transport only | Recently added intentionally per git log; reversing too soon |
| 6 | Drop Rust-only — language-agnostic LSP fabric | Strategic but speculative; depends on #2 (library crate) and is brainstorm-fodder, not ready-to-plan |
| 7 | Public hosted lspmux.dev (1M users, RA-as-a-service) | Subject-replacement: becomes a different organization; out of scope |
| 8 | Zero-Daemon (ephemeral RA per request) | Subject-replacement: explicitly inverts the project's core value prop |
| 9 | 600 tools (per-symbol generated tools) | LLM-routing-by-tool-name claim is unproven; enormous registry churn on edits |
| 10 | Multi-tenant zero-trust (capability tokens, sandboxing) | Strong but premature; needs sessions (#3) first |
| 11 | Team-shared cache via Tailscale | Strong but specialized; needs sessions (#3) and auth shim first |
| 12 | RA save states / rewind (emulator analogy) | Requires upstream RA changes; not actionable here |
| 13 | Smalltalk image-based daemon persistence | Overlaps with sessions (#3); more speculative |
| 14 | Library card catalog (persistent symbol index) | Duplicates RA's job; degraded-mode value real but minor |
| 15 | Telephone exchange / capacity-bound capacity logic | Overlaps with OTP supervisor; less specific |
| 16 | DNS-style tiered resolution for workspace roots | Mostly debug tool; folds into doctor (#6) |
| 17 | OTP supervisor tree for RA lifecycles | Real value but covered by launchd already; doctor (#6) makes it visible without restructuring |
| 18 | LSPMUX_CONFIG_PATH inferred default | Polish, not step-function; fold into doctor work |
| 19 | Project-memory typed schema crate | Off-scope for lspmux-cc (oh-my-claudecode handles this) |
| 20 | Surface hook failures to stderr/log (REV-009 standalone) | Already in-progress per recent activity; subsumed by doctor (#6) |
| 21 | In-flight request coalescing (moka) standalone | Subsumed if #4 (push subscriptions) lands; otherwise minor polish |
| 22 | CDN PoP map status tool | Subsumed by doctor (#6) |
| 23 | Pre-flight `lspmux-cc-doctor` standalone | Folded into distribution triad (#6) |
| 24 | Workspace-roaming session handoff standalone | Subsumed by sessions (#3) |

## Verification Plan (per chosen idea)

The user will pick one idea to take into `/ce-brainstorm` next. For whichever
is chosen:

- **#1 code actions:** `cargo test --manifest-path mcp-server/Cargo.toml` for
  parameter validation; manual e2e in Claude Code with a known fixable
  diagnostic (unused import → auto-fix); confirm the WorkspaceEdit shape
  matches RA upstream.
- **#2 library crate:** `nix flake check` and `just check` after the workspace
  split; verify integration tests can now exercise tools.rs paths.
- **#3 sessions:** `rust_server_status` should return session ID; manually
  reopen Claude Code with a different cwd and confirm the mismatch is detected.
- **#4 subscriptions:** edit a file, confirm push notification arrives without
  a polling tool call; check ring-buffer backlog on reconnect.
- **#5 multiplex axis:** spin up two worktrees on the same toolchain, confirm
  one RA process serves both via `rust_server_status` and `ps`.
- **#6 distribution triad:** clean macOS without Nix; `brew install`; daemon
  starts; socket auto-allowlisted; `lspmux-cc-doctor` returns green JSON;
  Claude Code sees diagnostics on a fresh Cargo project.
- **#7 traffic recorder:** trigger a known REV-009 silent failure; recover the
  failure trace from SQLite; replay it as a test fixture.

## Next Step

After `ExitPlanMode` approval, the user picks one of:
- Refine in conversation (add ideas, raise the bar, dig deeper on idea #N)
- Open and iterate in Proof (HITL review loop with collaborative comments)
- Brainstorm a selected idea (load `/ce-brainstorm` with the chosen idea as seed)
- Save and end (persist this artifact to `docs/ideation/2026-04-30-open-ideation.md`)
