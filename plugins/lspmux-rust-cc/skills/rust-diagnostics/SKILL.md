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

### `rust_server_status`
Check server health and confirm the active workspace root.
```
rust_server_status()
```

## Notes

- All file paths must be absolute.
- **Coordinate format:** `line` and `character` inputs are **zero-based** (first line = 0, first column = 0).
- **Output locations** (`file:line:col`) are **one-based**. To reuse an output location as input to another tool, subtract 1 from both line and column.
  - Example: `rust_goto_definition` returns `src/main.rs:42:5` → call next tool with `line=41, character=4`
- After file edits, rust-analyzer needs a moment to re-analyze. If diagnostics seem stale, wait a few seconds and retry.
- The lspmux server must be running (the session-start hook handles this automatically).
