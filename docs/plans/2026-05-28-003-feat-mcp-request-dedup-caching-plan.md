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

Two rapid `rust_diagnostics` calls on the same unchanged file fire two independent LSP requests to the shared rust-analyzer. There's no response cache and no in-flight coalescing at the MCP tool layer (distinct from `ensure_file_open()`'s content-hash dedup, which only suppresses redundant `didChange` notifications). This plan adds a `moka`-backed `ToolCache` wrapping tool dispatch: identical concurrent calls coalesce into one LSP request via `try_get_with`, recent results return from a short TTL cache, and the cache is invalidated whenever a `didChange` is sent. `rust_server_status` bypasses the cache (must be live).

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
- **R3.** The cache is invalidated when `ensure_file_open()` sends a `didChange`.
- **R4.** `rust_server_status` always returns live data (bypasses cache).
- **R5.** No cache poisoning from empty rust-analyzer responses during indexing.
- **R6.** Cache hit/miss counters are visible in telemetry (after REV-004, which has landed).
- **R7.** Tests verify coalescing behavior and cache invalidation.

---

## Key Technical Decisions

**KTD1 — `moka::future::Cache` with `try_get_with` (Option 1).** One crate, one primitive covers both in-flight coalescing and TTL caching. Add `moka = { version = "0.12", features = ["future"] }` to `mcp-server/Cargo.toml`. Rejected: manual `DashMap` + `broadcast` (≈100 lines, must hand-roll TTL/eviction); TTL-only cache (misses the concurrent case).

**KTD2 — Cache key `(tool_name: String, params_hash: u64)`.** `serde_json::Value` isn't `Hash`; serialize args to canonical JSON string, then hash to `u64`. Tool name namespaces the key.

**KTD3 — Workspace-wide invalidation via `invalidate_all()` on any `didChange`.** rust-analyzer's dependency graph is workspace-wide (editing `lib.rs` can change diagnostics anywhere), so per-file invalidation is unsafe. `invalidate_all()` is O(1) (timestamp-based, lazy cleanup).

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

**Approach:** `ToolCache` holds a `moka::future::Cache<(String, u64), Arc<CachedValue>>`. Expose `get_or_run(tool_name, params_json, init_future)` that builds the key (KTD2) and calls `try_get_with`. TTL from `LSPMUX_CACHE_TTL_SECS` (default 5s). Optional `weigher` bounding by estimated byte size, not just entry count (LSP responses can be large). Expose `invalidate_all()`.

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

**Goal:** Route tool dispatch through the cache, bypassing `rust_server_status`.

**Requirements:** R1, R2, R4.

**Dependencies:** U1.

**Files:**
- `mcp-server/src/tools.rs` (`RustAnalyzerTools` holds a `ToolCache`; `call_tool` ~715 wraps `tool_router.call`)
- `mcp-server/src/main.rs` (construct `ToolCache` and pass into `RustAnalyzerTools::new`)

**Approach:** In `call_tool`, if the tool is `rust_server_status`, dispatch directly (KTD4). Otherwise build the key from the tool name + serialized params and call `cache.get_or_run(..., async { tool_router.call(ctx).await })`. Wrap the result in `Arc` if needed (KTD7).

**Test scenarios:**
- Covers R4: `rust_server_status` is never served from cache (two calls produce two live dispatches).
- Covers R1/R2: repeated/concurrent `rust_diagnostics` on one unchanged file dispatch once.
- Integration: a cached tool returns identical payload on the cache hit.

**Verification:** Cached tools dispatch once within TTL; status bypasses.

### U3. Invalidate cache on `didChange`

**Goal:** Stale results never survive a document change.

**Requirements:** R3.

**Dependencies:** U1, U2.

**Files:**
- `mcp-server/src/lsp_client.rs` (`ensure_file_open` ~414-470: invoke an invalidation callback after a `didChange` is actually sent)
- wiring so `LspClient` can reach the `ToolCache` (callback/handle injected at construction)

**Approach:** When `ensure_file_open` sends a `didChange` (i.e., content hash changed), trigger `cache.invalidate_all()` (KTD3). Inject the cache handle/callback into `LspClient` at construction to avoid a layering cycle (a small `Arc<dyn Fn()>` or an `Arc<ToolCache>` reference).

**Execution note:** Add the failing invalidation test (change file → prior cached diagnostics not returned) before wiring the callback.

**Test scenarios:**
- Covers R3: cache a `rust_diagnostics` result, send a `didChange` for that file, assert the next call re-dispatches (cache miss).
- Edge: `ensure_file_open` on an *unchanged* file (hash match → no `didChange`) does NOT invalidate.
- Integration: change in file A invalidates cached diagnostics keyed on file B (workspace-wide invalidation, KTD3).

**Verification:** `didChange` clears the cache; no-op opens don't.

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
- Per-file or dependency-aware invalidation (intentionally rejected for now in favor of `invalidate_all()`).

Out of scope: caching at the `LspClient::request` layer (kept at the tool layer for clear keys and invalidation).

---

## Risks & Dependencies

- **Cache poisoning during indexing** (R5): empty diagnostics cached before RA is ready. Mitigated by KTD6; fuller fix depends on REV-010/AGENT-4 quiescence.
- **`CallToolResult` clonability** (KTD7): may need `Arc` wrapping; resolve when integrating U2.
- **Layering**: `LspClient` invalidating the tool-layer cache (U3) needs a callback to avoid a dependency cycle.
- New dependency `moka` (~50KB compile overhead) — acceptable, battle-tested.
- REV-004 (done) provides the telemetry substrate for U4.

---

## Verification

1. `cargo test --manifest-path mcp-server/Cargo.toml` covers coalescing (single init under concurrency), TTL hit, error-not-cached, empty-skip, and `didChange` invalidation.
2. `cargo build --manifest-path mcp-server/Cargo.toml` succeeds with `moka` added.
3. `rust_server_status` still returns live data (never cached) and now carries hit/miss counters.
4. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` clean.

---

## Sources & Research

- Origin todo: `todos/2026-03-18-mcp-request-deduplication-and-caching.md` (REV-008).
- moka docs: https://docs.rs/moka/latest/moka/future/struct.Cache.html ; CacheBuilder: https://docs.rs/moka/latest/moka/future/struct.CacheBuilder.html
- Verified locations: `call_tool` `mcp-server/src/tools.rs:715` (dispatch via `tool_router.call` ~731); `request()` `mcp-server/src/lsp_client.rs:270-314`; `ensure_file_open` `mcp-server/src/lsp_client.rs:414-470`; `ClientCapabilities` with serverStatus override `mcp-server/src/lsp_client.rs:236-240`; `moka`/`dashmap` absent from `mcp-server/Cargo.toml`.
- Related (done): REV-004 observability/attribution. Related: REV-010 (AGENT-4 quiescence) for the fuller poisoning fix.
