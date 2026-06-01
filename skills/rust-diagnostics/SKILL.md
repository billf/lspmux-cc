# Rust Diagnostics via lspmux

Use the MCP tools provided by `lspmux-rust-analyzer` to get rust-analyzer intelligence.

## Available Tools

### `rust_diagnostics`
Get compiler errors and warnings for a Rust file.
```
rust_diagnostics(file_path: "/absolute/path/to/file.rs")
```

### `rust_hover`
Get type signature and documentation at a position (zero-based line/character).
```
rust_hover(file_path: "/absolute/path/to/file.rs", line: 10, character: 5)
```

### `rust_goto_definition`
Find where a symbol is defined.
```
rust_goto_definition(file_path: "/absolute/path/to/file.rs", line: 10, character: 5)
```

### `rust_find_references`
Find all references to a symbol.
```
rust_find_references(file_path: "/absolute/path/to/file.rs", line: 10, character: 5)
```

### `rust_workspace_symbol`
Search for symbols (functions, structs, traits, etc.) by name across the entire workspace.
```
rust_workspace_symbol(query: "MyStruct")
```

### `rust_goto_implementation`
Find implementations of a trait or trait method (distinct from go-to-definition).
```
rust_goto_implementation(file_path: "/absolute/path/to/file.rs", line: 10, character: 5)
```

### `rust_document_symbols`
Outline a file's symbols (modules, functions, structs, impls) as a tree, without reading the file.
```
rust_document_symbols(file_path: "/absolute/path/to/file.rs")
```

### `rust_code_actions`
List the quick fixes and refactors rust-analyzer offers for a range. Returns each action's title, kind, and workspace edit as data — **edits are not applied**; apply them yourself.
```
rust_code_actions(file_path: "/absolute/path/to/file.rs", start_line: 10, start_character: 0, end_line: 10, end_character: 20)
```

### `rust_rename`
Compute the workspace edit to rename a symbol across the codebase. Returns per-file text edits as data — **nothing is written to disk**; apply the edits yourself.
```
rust_rename(file_path: "/absolute/path/to/file.rs", line: 10, character: 5, new_name: "new_symbol_name")
```

### `rust_call_hierarchy_incoming`
Find the callers of the function/method at a position.
```
rust_call_hierarchy_incoming(file_path: "/absolute/path/to/file.rs", line: 10, character: 5)
```

### `rust_call_hierarchy_outgoing`
Find the functions/methods called by the symbol at a position.
```
rust_call_hierarchy_outgoing(file_path: "/absolute/path/to/file.rs", line: 10, character: 5)
```

### `rust_expand_macro`
Expand the macro invocation at a position; returns the macro name and expanded source.
```
rust_expand_macro(file_path: "/absolute/path/to/file.rs", line: 10, character: 5)
```

### `rust_server_status`
Check server health and confirm the active workspace root.
```
rust_server_status()
```

### `rust_workspace_registry`
List all rust-analyzer instances the lspmux daemon is currently hosting (pid, workspace, idle time, client count). Use to debug workspace mismatches or confirm your workspace has a dedicated rust-analyzer.
```
rust_workspace_registry()
```

## Notes

- All file paths must be absolute.
- **Coordinate format:** `line` and `character` inputs are **zero-based** (first line = 0, first column = 0).
- **Output locations** (`file:line:col`) are **one-based**. To reuse an output location as input to another tool, subtract 1 from both line and column.
  - Example: `rust_goto_definition` returns `src/main.rs:42:5` → call next tool with `line=41, character=4`
- After file edits, rust-analyzer needs a moment to re-analyze. If diagnostics seem stale, wait a few seconds and retry.
- The lspmux server must be running. The session-start hook only reports status; the Rust MCP runtime owns bootstrap behavior.
