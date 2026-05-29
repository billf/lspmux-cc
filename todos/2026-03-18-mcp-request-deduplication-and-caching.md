---
status: pending
priority: p3
issue_id: "REV-008"
tags: [performance, caching, deduplication, mcp, moka]
dependencies: ["REV-004"]
---

# Add request deduplication and response caching at the MCP tool layer

## Problem Statement

Two rapid `rust_diagnostics` calls on the same unchanged file send two independent LSP requests to rust-analyzer. There's no response-level cache and no in-flight request coalescing. This is distinct from `ensure_file_open()`'s content-hash dedup (which prevents redundant `didChange` notifications, not LSP requests).

In practice, an agent might call the same tool multiple times in quick succession (retrying after a stale-looking result, or multiple parallel tool calls hitting the same file). Each call hits rust-analyzer independently, adding unnecessary load to the shared instance.

## Findings

- `mcp-server/src/tools.rs:616-623` (`call_tool`) dispatches every call directly through the `ToolRouter` to `LspClient` with no caching layer.
- `mcp-server/src/lsp_client.rs:247-291` (`request`) sends every LSP request independently; no dedup for identical in-flight requests.
- `mcp-server/src/lsp_client.rs:391-447` (`ensure_file_open`) has content-hash dedup that skips `didChange` for unchanged files, but this only prevents document sync overhead, not the actual LSP request.
- The `moka` crate (v0.12+, `"future"` feature) provides `try_get_with()` which gives built-in async coalescing: concurrent calls on the same cache key execute the init future exactly once; other callers wait and receive a clone. Errors are NOT cached (only `Ok` values are inserted).
- No ecosystem precedent exists for MCP-to-LSP response caching. lspmux-cc would be novel here.

## Proposed Solutions

### Option 1: moka `try_get_with` (recommended)

**Approach:** Add `moka = { version = "0.12", features = ["future"] }`. Create a `ToolCache` wrapper in `mcp-server/src/cache.rs`. Integration point: `RustAnalyzerTools::call_tool()` wrapping the existing `ToolRouter::call()`.

Cache key: `(tool_name: String, params_hash: u64)` from canonical JSON serialization of args. `serde_json::Value` doesn't implement `Hash`, so serialize to string first.

`invalidate_all()` on `didChange` (not per-file; RA's dependency graph means changing `lib.rs` invalidates diagnostics everywhere). Don't cache `rust_server_status` (must reflect live state).

**Pros:**
- One crate, one primitive, handles both in-flight coalescing AND TTL caching
- Errors NOT cached (`try_get_with` only inserts on `Ok`); failed requests retry naturally
- TinyLFU eviction, configurable TTL, bounded by count or weighted size
- `invalidate_all()` is O(1) (timestamp-based, lazy cleanup)

**Cons:**
- New dependency (~50KB compile overhead)
- `CallToolResult` must be `Clone` (may need `Arc` wrapper if rmcp's type isn't Clone)
- Cache poisoning during RA indexing: empty diagnostics get cached

**Effort:** 4-6 hours

**Risk:** Low

---

### Option 2: Manual `DashMap` + `broadcast::Sender`

**Approach:** Hand-roll coalescing with `DashMap<CacheKey, Arc<broadcast::Sender<Result>>>`. First caller inserts sender, makes request, broadcasts result. Subsequent callers subscribe to existing broadcast.

**Pros:**
- No new dependency (uses `tokio::sync::broadcast` and `dashmap`)
- Full control over coalescing logic

**Cons:**
- More code (~100 lines vs ~30 for moka)
- Must implement TTL and eviction manually
- Must handle edge cases (sender dropped, broadcast lag)

**Effort:** 1-2 days

**Risk:** Medium

---

### Option 3: TTL-only cache (no coalescing)

**Approach:** Cache recent tool responses with configurable TTL. No in-flight dedup.

**Pros:**
- Simplest implementation
- Catches the "retry same tool" case

**Cons:**
- Misses the concurrent-request case (two parallel tool calls still both hit RA)
- Why bother if moka gives coalescing for free?

**Effort:** 2-4 hours

**Risk:** Low

## Recommended Action

Implement Option 1 (moka). The `try_get_with` pattern is the cleanest solution, and the crate is battle-tested in production Rust async services. Start with a 5-second TTL and `invalidate_all()` on `didChange`. Add cache hit/miss counters once REV-004 (observability) lands.

## Technical Details

**Affected files:**
- `mcp-server/Cargo.toml` — add `moka = { version = "0.12", features = ["future"] }`
- `mcp-server/src/cache.rs` — new file: `ToolCache` struct wrapping `moka::future::Cache`
- `mcp-server/src/tools.rs` — integrate `ToolCache` into `call_tool()`
- `mcp-server/src/lsp_client.rs` — add cache invalidation callback after `didChange` in `ensure_file_open()`
- `mcp-server/src/lib.rs` — add `pub mod cache;`

**Related components:**
- REV-004 (observability) — cache hit/miss counters depend on metrics infrastructure
- `rust_server_status` — should bypass cache (always live)

**Key design decisions:**
- Cache key: `(tool_name, params_hash)` where params_hash is `u64` from canonical JSON serialization
- Use `invalidate_all()` on any `didChange`, not per-file invalidation (RA dependency graph is workspace-wide)
- Use moka's `weigher` to bound by estimated byte size, not just entry count (LSP responses can be large)
- Consider skipping cache for responses that look empty during RA's first 30 seconds (cache poisoning mitigation)

## Resources

- **Crate docs:** [moka::future::Cache](https://docs.rs/moka/latest/moka/future/struct.Cache.html)
- **Crate docs:** [moka CacheBuilder](https://docs.rs/moka/latest/moka/future/struct.CacheBuilder.html)
- **Related todo:** `todos/archive/2026-03-18-observability-and-client-attribution.md` (REV-004, done)
- **Related review:** `todos/archive/review-2026-03-18-p2-important.md` (PERF-1, PERF-2)

## Acceptance Criteria

- [ ] Duplicate MCP tool calls within a configurable TTL window return cached results
- [ ] Concurrent identical tool calls coalesce into a single LSP request
- [ ] Cache is invalidated when `ensure_file_open()` sends a `didChange` notification
- [ ] `rust_server_status` always returns live data (bypasses cache)
- [ ] Cache hit/miss counters are visible in telemetry (after REV-004 lands)
- [ ] No cache poisoning from empty RA responses during indexing
- [ ] Tests verify coalescing behavior and cache invalidation

## Work Log

### 2026-03-18 - Initial discovery

**By:** Claude Code (deep review follow-up)

**Actions:**
- Identified the gap: no response-level caching or in-flight coalescing in the MCP tool layer
- Researched crate options: moka, quick_cache, cached, mini-moka, manual DashMap+broadcast
- Confirmed moka v0.12+'s `try_get_with()` is the best fit (async coalescing + TTL in one primitive)
- Validated that `invalidate_all()` is the correct invalidation strategy (not per-file)

**Learnings:**
- No ecosystem precedent for MCP-to-LSP response caching; this would be novel
- `try_get_with` only caches `Ok` results, so LSP failures naturally retry
- `CallToolResult` from rmcp may need an `Arc` wrapper for `Clone`

---

## Notes

- This TODO depends on REV-004 landing first for metrics. The caching itself can be built independently, but measuring its effectiveness requires counters.
- The 5-second default TTL is a starting point. It should be tunable via env var (`LSPMUX_CACHE_TTL_SECS`) for experimentation.
- If the user's RA is still indexing, consider detecting the "quiescent" state from `experimental/serverStatus` (REV-004) and disabling cache until quiescent.
