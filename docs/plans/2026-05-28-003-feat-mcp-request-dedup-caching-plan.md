---
title: "feat: MCP tool-layer request deduplication and response caching (REV-008)"
status: active
date: 2026-05-28
type: feat
issue_id: REV-008
origin: todos/2026-03-18-mcp-request-deduplication-and-caching.md
---

# feat: MCP tool-layer request deduplication and response caching (REV-008)

## Summary

Two rapid `rust_diagnostics` calls on the same unchanged file fire two independent LSP requests to the shared rust-analyzer. There's no response cache and no in-flight coalescing at the MCP tool layer (distinct from `ensure_file_open()`'s content-hash dedup, which only suppresses redundant `didChange` notifications). This plan adds a `moka`-backed `ToolCache` wrapping tool dispatch: identical concurrent calls coalesce into one LSP request via `try_get_with`, recent results return from a short TTL cache. File sync (`ensure_file_open()`) runs at the `call_tool` layer *before* the cache lookup, so a hit can never bypass change detection. Invalidation is via a monotonic workspace sync generation stamped into the cache key (bumped on `didOpen`/`didChange`), not `invalidate_all()`. `rust_server_status` bypasses the cache (must be live).

---

## Problem Frame

- `mcp-server/src/tools.rs` `call_tool` (~715, dispatching through `tool_router.call` at ~731) sends every call straight to `LspClient` with no caching layer.
- `mcp-server/src/lsp_client.rs` `request()` (~270-314) sends every LSP request independently; no dedup for identical in-flight requests.
- `ensure_file_open()` (`mcp-server/src/lsp_client.rs:414-470`) has content-hash dedup that skips `didChange` for unchanged files — but that only saves document-sync overhead, not the LSP request itself.
- An agent may call the same tool repeatedly in quick succession (retry after a stale-looking result, or parallel tool calls on the same file). Each call independently loads the shared rust-analyzer.
- `moka` and `dashmap` are **not** currently dependencies (verified in `mcp-server/Cargo.toml`).
- `moka` v0.12+ (`future` feature) provides `try_get_with()`: concurrent calls on one key run the init future exactly once; others await and receive a clone. Only `Ok` values are cached — failures retry naturally.

---

## Requirements

- **R1.** Duplicate MCP tool calls within a configurable TTL window return cached results.
- **R2.** Concurrent identical tool calls coalesce into a single LSP request.
- **R3.** Cached results never survive a content change: `ensure_file_open()` runs before the cache lookup, and the sync generation it bumps on `didOpen`/`didChange` is part of the cache key, so a changed file always misses.
- **R4.** `rust_server_status` always returns live data (bypasses cache).
- **R5.** No cache poisoning from empty rust-analyzer responses during indexing.
- **R6.** Cache hit/miss counters are visible in telemetry (after REV-004, which has landed).
- **R7.** Tests verify coalescing behavior and cache invalidation.

---

## Key Technical Decisions

**KTD1 — `moka::future::Cache` with `try_get_with` (Option 1).** One crate, one primitive covers both in-flight coalescing and TTL caching. Add `moka = { version = "0.12", features = ["future"] }` to `mcp-server/Cargo.toml`. Rejected: manual `DashMap` + `broadcast` (≈100 lines, must hand-roll TTL/eviction); TTL-only cache (misses the concurrent case).

**KTD2 — Cache key `(tool_name: String, params_hash: u64, sync_gen: u64)`.** `serde_json::Value` isn't `Hash`; serialize args to canonical JSON string, then hash to `u64`. Tool name namespaces the key. `sync_gen` is the workspace sync generation (KTD3), read *after* the pre-sync `ensure_file_open()` so a content change is reflected in the key before the lookup.

**KTD3 — Workspace-wide invalidation via a generation-stamped key.** `LspClient` holds a monotonic `AtomicU64` bumped whenever `ensure_file_open()` sends a `didOpen`/`didChange`. The current value is stamped into every cache key (KTD2). A change bumps the generation, so all subsequent keys miss and re-dispatch. rust-analyzer's dependency graph is workspace-wide (editing `lib.rs` can change diagnostics anywhere), so the generation is global, not per-file. This replaces `moka::invalidate_all()`, which has a `try_get_with` race: an in-flight init that inserts *after* the invalidation timestamp survives, so a call arriving post-edit could coalesce onto a pre-edit in-flight result. Stamping the generation into the key gives that late call a distinct key, so it never coalesces onto the stale result. Stale entries become unreachable and are reaped by TTL + bounded capacity.

**KTD4 — `rust_server_status` bypasses the cache.** It must reflect live daemon/workspace state. The cache wrapper checks the tool name and skips caching for it.

**KTD5 — Module placement: `mcp-server/src/cache.rs` in the library crate.** `tools.rs` is currently binary-only (ARCH-1 lib split not done), but `cache.rs` is self-contained and belongs in `lib.rs` (`pub mod cache;`) so it's unit-testable without the binary. The integration point (`call_tool`) stays in `tools.rs` in the binary and imports `lspmux_cc_mcp::cache::ToolCache`.

**KTD6 — Cache-poisoning mitigation.** Skip caching responses that look empty while rust-analyzer is non-quiescent. Reuse the `experimental/serverStatus` signal already wired into `ClientCapabilities` (`mcp-server/src/lsp_client.rs:236-240`); REV-010/AGENT-4 formalizes quiescence, but a minimal "empty result + indexing → don't cache" guard ships here.

**KTD7 — `CallToolResult` clone-ability.** `try_get_with` requires the cached value be `Clone`. If rmcp's `CallToolResult` isn't `Clone`, wrap in `Arc` for the cache value and clone the `Arc`.

---

## Implementation Units

### U1. Add `ToolCache` wrapper over `moka::future::Cache`

**Goal:** A self-contained cache type providing keyed `try_get_with` coalescing + TTL.

**Requirements:** R1, R2, R5, R7.

**Dependencies:** none.

**Files:**
- `mcp-server/Cargo.toml` (add `moka` with `future` feature)
- `mcp-server/src/cache.rs` (new: `ToolCache`, key hashing, TTL config, optional weigher)
- `mcp-server/src/lib.rs` (`pub mod cache;`)
- `mcp-server/src/cache.rs` unit tests

**Approach:** `ToolCache` holds a `moka::future::Cache<(String, u64, u64), Arc<CachedValue>>`. Expose `get_or_run(tool_name, params_json, sync_gen, init_future)` that builds the key (KTD2) and calls `try_get_with`. TTL from `LSPMUX_CACHE_TTL_SECS` (default 5s). A bounded `max_capacity` (and optional `weigher` by estimated byte size, since LSP responses can be large) reaps entries that the generation stamp has made unreachable — there's no `invalidate_all` (KTD3).

**Patterns to follow:** moka `try_get_with` docs (https://docs.rs/moka). Existing config-via-env style in `mcp-server/src/telemetry.rs` (`from_env`).

**Test scenarios:**
- Covers R2: two concurrent `get_or_run` calls on the same key run the init future exactly once (use an atomic counter in the init closure).
- Covers R1: a second call within TTL returns the cached clone without re-running init.
- Edge: distinct keys (different tool or params hash) do not collide.
- Error path: an init future returning `Err` is NOT cached; the next call re-runs (assert counter increments again).
- Edge (R5): a result flagged empty-during-indexing is not inserted.
- TTL: after TTL expiry, init runs again.

**Verification:** Coalescing, TTL, error-not-cached, and empty-skip behaviors proven by unit tests.

### U2. Integrate `ToolCache` into `call_tool`

**Goal:** Route tool dispatch through the cache, syncing the file before the lookup and bypassing `rust_server_status`.

**Requirements:** R1, R2, R3, R4.

**Dependencies:** U1, U3 (needs `LspClient::sync_generation()`).

**Files:**
- `mcp-server/src/tools.rs` (`RustAnalyzerTools` holds a `ToolCache`; `call_tool` (`tools.rs:1488-1550`) pre-syncs then wraps `tool_router.call`)
- `mcp-server/src/main.rs` (construct `ToolCache` and pass into `RustAnalyzerTools::new`)

**Approach:** In `call_tool`:
1. If the tool is `rust_server_status`, dispatch directly (KTD4).
2. **Pre-sync before the cache lookup.** Extract `file_path` generically from `request.arguments` (every file-bearing param struct — `FileParam`, `PositionParam`, `RangeParam`, `RenameParam` — uses the field name `file_path`), validate via the existing `validate_file_path` helper (`tools.rs:44-59`), and call `self.lsp.ensure_file_open(fp)`. This re-reads the file, and on a content change sends `didChange` and bumps the sync generation *before* the key is built — so a hit can never bypass change detection. Tools with no `file_path` (`rust_workspace_symbol`, `rust_workspace_registry`) skip this step. If `ensure_file_open` returns `Err`, bypass the cache and dispatch directly so the handler produces the canonical live error (errors aren't cached anyway).
3. Read `self.lsp.sync_generation()` (after the pre-sync), build the key `(tool_name, params_hash, sync_gen)`, and call `cache.get_or_run(key, async { tool_router.call(ctx).await })`. Wrap the result in `Arc` if needed (KTD7).

The in-handler `ensure_file_open()` calls stay as-is: after the pre-sync the content hash already matches, so the handler's call is a no-op. This keeps each handler independently correct and minimizes the diff.

**Test scenarios:**
- Covers R4: `rust_server_status` is never served from cache (two calls produce two live dispatches).
- Covers R1/R2: repeated/concurrent `rust_diagnostics` on one unchanged file dispatch once.
- Covers R3: change the file on disk between two identical `rust_diagnostics` calls → the pre-sync bumps the generation → the second call is a cache miss returning fresh data.
- Integration: a cached tool returns identical payload on the cache hit.

**Verification:** Cached tools dispatch once within TTL on an unchanged file; an on-disk change forces a fresh dispatch; status bypasses.

### U3. Workspace sync generation

**Goal:** Stale results never survive a document change.

**Requirements:** R3.

**Dependencies:** none (self-contained `LspClient` change; U2 consumes it).

**Files:**
- `mcp-server/src/lsp_client.rs` (`LspClient` (`lsp_client.rs:43-59`) gains an `AtomicU64`; `ensure_file_open` (`lsp_client.rs:409-465`) bumps it on `didOpen`/`didChange`; add a `sync_generation()` reader)

**Approach:** Add a monotonic `AtomicU64` field to `LspClient`. Increment it (`fetch_add(1, Ordering::Relaxed)`) in the two branches of `ensure_file_open` that actually send a notification — the `didChange` branch (`lsp_client.rs:429-446`) and the `didOpen` branch (`lsp_client.rs:448-464`) — never in the hash-match early return. Expose `pub fn sync_generation(&self) -> u64`. U2 stamps this value into the cache key (KTD2/KTD3), so a change makes every later key miss; no callback into the cache and no `invalidate_all`, so there's no layering cycle.

**Execution note:** Add the failing staleness test (change file → prior cached diagnostics not returned) before wiring the generation bump.

**Test scenarios:**
- Covers R3: cache a `rust_diagnostics` result, change the file (so `ensure_file_open` sends a `didChange`), assert the next call re-dispatches (cache miss) because the generation bumped.
- Edge: `ensure_file_open` on an *unchanged* file (hash match → no `didChange`) does NOT bump the generation, so a cached result is still served.
- Integration: editing file A bumps the generation, so a later query on file B re-dispatches (workspace-wide freshness, KTD3).

**Verification:** A sent `didChange`/`didOpen` bumps the generation; no-op opens don't.

### U4. Cache hit/miss telemetry counters

**Goal:** Make cache effectiveness measurable.

**Requirements:** R6.

**Dependencies:** U1, U2.

**Files:**
- `mcp-server/src/telemetry.rs` (hit/miss counters)
- `mcp-server/src/cache.rs` or `tools.rs` (increment on hit vs init-run)
- `mcp-server/src/bootstrap.rs` `RuntimeStatus` or `rust_server_status` response (surface counts)

**Approach:** Increment a hit counter when `try_get_with` returns without running init, a miss counter when init runs. Reuse the `telemetry.rs` accounting style. Surface in status (additive field).

**Test scenarios:**
- Covers R6: after one miss + one hit, counters read 1/1.
- Edge: coalesced concurrent calls count as one miss, not N.

**Verification:** Counters reflect hit/miss; visible via status.

### U5. Tests and docs

**Goal:** Lock behavior and document the TTL env var.

**Requirements:** R7.

**Dependencies:** U1-U4.

**Files:**
- `mcp-server/tests/` or in-module tests covering coalescing + invalidation end-to-end
- `docs/hosts/*.md` note on `LSPMUX_CACHE_TTL_SECS` (default 5s, tunable)

**Test scenarios:**
- Covers R7: an end-to-end test exercising dispatch → cache hit → `didChange` → miss.

**Verification:** Tests green; env var documented.

---

## Scope Boundaries

In scope: response cache + in-flight coalescing at the MCP tool layer, `didChange` invalidation, status bypass, poisoning guard, hit/miss counters, tests, TTL doc.

### Deferred to Follow-Up Work
- Full quiescence detection from `experimental/serverStatus` / `$/progress` (REV-010 / AGENT-4). This plan ships only a minimal empty-during-indexing guard.
- Per-file or dependency-aware invalidation (intentionally rejected for now in favor of a single workspace-global sync generation, KTD3).

Out of scope: caching at the `LspClient::request` layer (kept at the tool layer for clear keys and invalidation).

---

## Risks & Dependencies

- **Cache poisoning during indexing** (R5): empty diagnostics cached before RA is ready. Mitigated by KTD6; fuller fix depends on REV-010/AGENT-4 quiescence.
- **`CallToolResult` clonability** (KTD7): may need `Arc` wrapping; resolve when integrating U2.
- New dependency `moka` (~50KB compile overhead) — acceptable, battle-tested.
- REV-004 (done) provides the telemetry substrate for U4.

---

## Verification

1. `cargo test --manifest-path mcp-server/Cargo.toml` covers coalescing (single init under concurrency), TTL hit, error-not-cached, empty-skip, generation-based invalidation, and the on-disk-change-between-calls staleness case (edit a file between two identical calls → the second misses and returns fresh data).
2. `cargo build --manifest-path mcp-server/Cargo.toml` succeeds with `moka` added.
3. `rust_server_status` still returns live data (never cached) and now carries hit/miss counters.
4. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` clean.

---

## Sources & Research

- Origin todo: `todos/2026-03-18-mcp-request-deduplication-and-caching.md` (REV-008).
- moka docs: https://docs.rs/moka/latest/moka/future/struct.Cache.html ; CacheBuilder: https://docs.rs/moka/latest/moka/future/struct.CacheBuilder.html
- Verified locations: `call_tool` `mcp-server/src/tools.rs:715` (dispatch via `tool_router.call` ~731); `request()` `mcp-server/src/lsp_client.rs:270-314`; `ensure_file_open` `mcp-server/src/lsp_client.rs:414-470`; `ClientCapabilities` with serverStatus override `mcp-server/src/lsp_client.rs:236-240`; `moka`/`dashmap` absent from `mcp-server/Cargo.toml`.
- Related (done): REV-004 observability/attribution. Related: REV-010 (AGENT-4 quiescence) for the fuller poisoning fix.
