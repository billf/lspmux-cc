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

## Notes

- All file paths must be absolute.
- Line and character positions are zero-based.
- After file edits, rust-analyzer needs a moment to re-analyze. If diagnostics seem stale, wait a few seconds and retry.
- The lspmux server must be running (the session-start hook handles this automatically).
