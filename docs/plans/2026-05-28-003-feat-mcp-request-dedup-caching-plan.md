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

Two rapid `rust_diagnostics` calls on the same unchanged file fire two independent LSP requests to the shared rust-analyzer. There's no response cache and no in-flight coalescing at the MCP tool layer (distinct from `ensure_file_open()`'s content-hash dedup, which only suppresses redundant `didChange` notifications). This plan adds a `moka`-backed `ToolCache` wrapping tool dispatch: identical concurrent calls coalesce into one LSP request via `try_get_with`, and recent results return from a short TTL cache. File sync runs before cache lookup, and a monotonic workspace sync generation is part of the key, so a hit cannot bypass change detection. `rust_server_status` bypasses the cache because it must be live.

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
- **R3.** Cached results never survive a content change: `ensure_file_open()` runs before lookup, and the sync generation it bumps on `didOpen`/`didChange` is part of the cache key.
- **R4.** `rust_server_status` always returns live data (bypasses cache).
- **R5.** No cache poisoning from empty rust-analyzer responses during indexing.
- **R6.** Cache hit/miss counters are visible in telemetry (after REV-004, which has landed).
- **R7.** Tests verify coalescing behavior and cache invalidation.

---

## Key Technical Decisions

**KTD1 — `moka::future::Cache` with `try_get_with` (Option 1).** One crate, one primitive covers both in-flight coalescing and TTL caching. Add `moka = { version = "0.12", features = ["future"] }` to `mcp-server/Cargo.toml`. Rejected: manual `DashMap` + `broadcast` (≈100 lines, must hand-roll TTL/eviction); TTL-only cache (misses the concurrent case).

**KTD2 — Cache key `(tool_name, canonical_params, sync_generation)`.** Use a canonical parameter representation rather than a lossy hash: collisions must be impossible, not merely improbable. `sync_generation` is read after pre-sync, so a content change is reflected before lookup.

**KTD3 — Workspace-wide freshness via a generation-stamped key.** `LspClient` holds a monotonic `AtomicU64` bumped whenever `ensure_file_open()` sends `didOpen` or `didChange`. The current value is stamped into every cache key. A change therefore gives every later request a distinct key; it cannot coalesce onto a pre-edit in-flight result. This avoids the `invalidate_all()` race in which an old in-flight initializer inserts after invalidation. Stale entries become unreachable and are reaped by TTL plus bounded capacity.

**KTD4 — `rust_server_status` bypasses the cache.** It must reflect live daemon/workspace state. The cache wrapper checks the tool name and skips caching for it.

**KTD5 — Keep cache mechanics independent of tool dispatch.** Put the cache in `mcp-server/src/cache.rs` and export it from `lib.rs`, so its concurrency and freshness behavior can be unit-tested without the MCP router. Integrate it at `call_tool`.

**KTD6 — Cache-poisoning mitigation.** Skip caching responses that look empty while rust-analyzer is non-quiescent. The shipped readiness signal exposes this state; retain the conservative guard unless an end-to-end fixture proves that a particular empty response is final.

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

**Approach:** `ToolCache` holds a `moka::future::Cache<CacheKey, Arc<CachedValue>>`, where `CacheKey` contains the tool name, canonical parameters, and sync generation. Expose `get_or_run(key, init_future)` backed by `try_get_with`. Configure a short TTL and bounded capacity; do not expose `invalidate_all()`.

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

**Goal:** Route eligible read-only file tools through the cache, syncing files before lookup and bypassing live or mutable requests.

**Requirements:** R1, R2, R3, R4.

**Dependencies:** U1, U3.

**Files:**
- `mcp-server/src/tools.rs` (`RustAnalyzerTools` holds a `ToolCache`; `call_tool` ~715 wraps `tool_router.call`)
- `mcp-server/src/main.rs` (construct `ToolCache` and pass into `RustAnalyzerTools::new`)

**Approach:** Maintain an explicit allowlist of cacheable read-only query tools; do not infer eligibility from `file_path`. Before lookup, validate and synchronize a file-bearing request. Then read `sync_generation`, build the key, and call `cache.get_or_run`. Keep handler-level synchronization as a harmless hash-match no-op so handlers remain independently correct. Bypass status, the workspace registry, and operations that can observe changing global state. If pre-sync fails, dispatch directly to preserve the handler's canonical error.

**Test scenarios:**
- Covers R4: `rust_server_status` is never served from cache (two calls produce two live dispatches).
- Covers R1/R2: repeated/concurrent `rust_diagnostics` on one unchanged file dispatch once.
- Covers R3: changing a file between identical diagnostics calls bumps the generation, so the second call misses and returns fresh data.
- Integration: a cached tool returns identical payload on the cache hit.

**Verification:** An unchanged request dispatches once within TTL; an on-disk change forces a fresh dispatch; live tools bypass.

### U3. Workspace sync generation

**Goal:** Stale results never survive a document change.

**Requirements:** R3.

**Dependencies:** none.

**Files:**
- `mcp-server/src/lsp_client.rs` (add an `AtomicU64`, bump it when `ensure_file_open()` sends `didOpen` or `didChange`, and expose a reader)

**Approach:** Increment the generation only after a document-sync notification is sent, never on a hash-match early return. U2 reads it after pre-sync and stamps it into the key. This keeps `LspClient` independent of the tool-layer cache.

**Execution note:** Add the failing staleness test (change file → prior cached diagnostics not returned) before wiring the generation bump.

**Test scenarios:**
- Covers R3: cache diagnostics, change the file, then assert the next call re-dispatches because the generation changed.
- Edge: an unchanged file does not bump the generation, so its cached result remains eligible.
- Integration: editing file A changes the generation, so a later query for file B re-dispatches.

**Verification:** Sent `didOpen`/`didChange` notifications bump the generation; hash-match opens do not.

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

In scope: response cache + in-flight coalescing for an explicit query-tool allowlist, pre-lookup file sync, generation-based workspace freshness, live-tool bypasses, poisoning guard, hit/miss counters, tests, and configuration documentation.

### Deferred to Follow-Up Work
- Additional readiness signals such as `$/progress`; the shipped server-status readiness is sufficient for this conservative guard.
- Per-file or dependency-aware freshness (intentionally rejected for now in favor of one workspace-global sync generation).

Out of scope: caching at the `LspClient::request` layer (kept at the tool layer for clear keys and invalidation).

---

## Risks & Dependencies

- **Cache poisoning during indexing** (R5): empty diagnostics may be cached before RA is ready. Mitigate with the shipped readiness signal; retain short TTLs and an end-to-end fixture.
- **`CallToolResult` clonability** (KTD7): may need `Arc` wrapping; resolve when integrating U2.
- **Eligibility**: caching every tool by default risks serving stale global state. The U2 allowlist starts small and expands only with a freshness argument and test.
- New dependency `moka` adds compile and runtime complexity; admit it only if a benchmark shows meaningful repeated-call savings.
- Existing telemetry provides the baseline for U4; cache counters remain new work.

---

## Verification

1. `cargo test --manifest-path mcp-server/Cargo.toml` covers coalescing, TTL hit, error-not-cached, empty-while-indexing skip, generation freshness, and an on-disk-change-between-calls regression.
2. A focused benchmark demonstrates a material repeated-call saving before the dependency is retained.
3. `rust_server_status` remains live and exposes hit/miss counters.
4. `cargo clippy --manifest-path mcp-server/Cargo.toml --all-targets -- -W clippy::nursery -W clippy::pedantic` stays clean.

---

## Sources & Research

- Origin todo: `todos/2026-03-18-mcp-request-deduplication-and-caching.md` (REV-008).
- moka docs: https://docs.rs/moka/latest/moka/future/struct.Cache.html ; CacheBuilder: https://docs.rs/moka/latest/moka/future/struct.CacheBuilder.html
- Verified at audit: `call_tool` is in `mcp-server/src/tools.rs`; `ensure_file_open` is in `mcp-server/src/lsp_client.rs`; `moka` and `dashmap` are absent from `mcp-server/Cargo.toml`. The exact offsets are deliberately omitted because the tool surface evolves quickly.
- Related: existing telemetry and the shipped readiness signal.
