//! Meta-integration test: two LSP clients share a single rust-analyzer via lspmux.
//!
//! This test proves the core value proposition of lspmux: multiple LSP clients
//! (simulating Claude Code + Neovim) both talk to a **single** rust-analyzer
//! instance through the mux, operating on the MCP server's own source code.
//!
//! # Prerequisites
//!
//! - `lspmux` binary on PATH (or built via `nix build .#lspmux`)
//! - `rust-analyzer` binary on PATH
//!
//! # Running
//!
//! ```sh
//! cargo test --manifest-path mcp-server/Cargo.toml -- --ignored
//! # or: just integration-test
//! ```

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;

use lspmux_cc_mcp::lsp_client::LspClient;
use tokio::process::Command;
use tokio::time::sleep;

/// Find the line number (0-indexed) of a pattern in a file.
#[allow(clippy::cast_possible_truncation)]
fn find_line(path: &Path, pattern: &str) -> Option<u32> {
    let content = std::fs::read_to_string(path).ok()?;
    content
        .lines()
        .position(|l| l.contains(pattern))
        .map(|n| n as u32)
}

/// Find the column (0-indexed) where `needle` starts within the first line matching `pattern`.
#[allow(clippy::cast_possible_truncation)]
fn find_column(path: &Path, pattern: &str, needle: &str) -> Option<u32> {
    let content = std::fs::read_to_string(path).ok()?;
    content
        .lines()
        .find(|l| l.contains(pattern))
        .and_then(|line| line.find(needle).map(|c| c as u32))
}

/// Check if a binary exists on PATH.
fn binary_exists(name: &str) -> bool {
    StdCommand::new("which")
        .arg(name)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Count direct child processes of `parent_pid` whose command contains `needle`.
///
/// Panics if the `ps` command fails, since that indicates a broken test environment.
fn count_direct_children_named(parent_pid: u32, needle: &str) -> usize {
    let output = StdCommand::new("ps")
        .args(["-Ao", "ppid=,comm="])
        .output()
        .expect("failed to run `ps` — is it available on PATH?");
    assert!(
        output.status.success(),
        "ps exited with non-zero status: {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let mut parts = trimmed.split_whitespace();
            let ppid = parts.next()?.parse::<u32>().ok()?;
            let cmd = parts.next()?;
            Some((ppid, cmd))
        })
        .filter(|(ppid, cmd)| *ppid == parent_pid && cmd.contains(needle))
        .count()
}

/// Find a free TCP port by binding to port 0.
fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind to free port");
    listener
        .local_addr()
        .expect("failed to get local addr")
        .port()
}

/// Get the absolute path to the mcp-server workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Write a lspmux config file that uses the given port, in a directory structure
/// that `directories::ProjectDirs` will find when `HOME` is set to `home_dir`.
fn write_lspmux_config(home_dir: &Path, port: u16) {
    // On macOS: ~/Library/Application Support/lspmux/config.toml
    // On Linux: ~/.config/lspmux/config.toml
    let config_dir = if cfg!(target_os = "macos") {
        home_dir.join("Library/Application Support/lspmux")
    } else {
        home_dir.join(".config/lspmux")
    };
    std::fs::create_dir_all(&config_dir).expect("failed to create config dir");

    let config_content = format!(
        r#"listen = ["127.0.0.1", {port}]
connect = ["127.0.0.1", {port}]
"#
    );
    std::fs::write(config_dir.join("config.toml"), config_content)
        .expect("failed to write lspmux config");
}

/// Wait for a TCP port to become connectable, with timeout.
async fn wait_for_port(port: u16, timeout_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    while tokio::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
#[ignore = "requires lspmux + rust-analyzer binaries"]
#[allow(clippy::too_many_lines)]
async fn two_clients_share_single_rust_analyzer() {
    // ── 1. Check prerequisites ──────────────────────────────────────────
    if !binary_exists("lspmux") {
        eprintln!("SKIP: lspmux binary not found on PATH");
        return;
    }
    if !binary_exists("rust-analyzer") {
        eprintln!("SKIP: rust-analyzer binary not found on PATH");
        return;
    }

    let lspmux_bin = "lspmux";
    let ra_bin = "rust-analyzer";
    let ws_root = workspace_root();
    let ws_root_str = ws_root.to_str().expect("workspace root is valid UTF-8");

    // ── 2. Set up isolated lspmux instance ──────────────────────────────
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let fake_home = tmp.path();
    let port = find_free_port();

    write_lspmux_config(fake_home, port);

    let home_str = fake_home.to_str().expect("temp dir is valid UTF-8");

    // Start isolated lspmux server
    let mut server_proc = Command::new(lspmux_bin)
        .arg("server")
        .env("HOME", home_str)
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn lspmux server");

    // Wait for the server to start listening
    assert!(
        wait_for_port(port, 10).await,
        "lspmux server did not start listening on port {port} within 10s"
    );

    let env = [("HOME", home_str)];

    // ── 3. Create two LSP clients ───────────────────────────────────────
    let client_a = LspClient::new_with_env(lspmux_bin, ra_bin, Some(ws_root_str), &env)
        .await
        .expect("Client A: failed to initialize LSP client");

    let client_b = LspClient::new_with_env(lspmux_bin, ra_bin, Some(ws_root_str), &env)
        .await
        .expect("Client B: failed to initialize LSP client");

    // ── 4. Both open the same file ──────────────────────────────────────
    let target_file = ws_root.join("src/lsp_client.rs");
    let target_file_str = target_file.to_str().expect("file path is valid UTF-8");

    client_a
        .ensure_file_open(target_file_str)
        .await
        .expect("Client A: failed to open file");
    client_b
        .ensure_file_open(target_file_str)
        .await
        .expect("Client B: failed to open file");

    // Give rust-analyzer a moment to index the workspace.
    // This is inherently racy — ra may need time to load, especially on first run.
    sleep(Duration::from_secs(5)).await;

    // ── 5. Dynamic line discovery ───────────────────────────────────────
    let struct_line = find_line(&target_file, "pub struct LspClient")
        .expect("could not find 'pub struct LspClient' in lsp_client.rs");
    let struct_col = find_column(&target_file, "pub struct LspClient", "LspClient")
        .expect("could not find 'LspClient' column");

    let fn_line = find_line(&target_file, "pub fn file_uri")
        .expect("could not find 'pub fn file_uri' in lsp_client.rs");
    let fn_col = find_column(&target_file, "pub fn file_uri", "file_uri")
        .expect("could not find 'file_uri' column");

    // ── 6. Both hover on LspClient struct ───────────────────────────────
    let hover_a = client_a
        .hover(target_file_str, struct_line, struct_col)
        .await
        .expect("Client A: hover failed");
    let hover_b = client_b
        .hover(target_file_str, struct_line, struct_col)
        .await
        .expect("Client B: hover failed");

    let hover_text_a = hover_a
        .as_ref()
        .map(|h| format!("{h:?}"))
        .unwrap_or_default();
    let hover_text_b = hover_b
        .as_ref()
        .map(|h| format!("{h:?}"))
        .unwrap_or_default();

    assert!(
        hover_text_a.contains("LspClient"),
        "Client A hover should mention LspClient, got: {hover_text_a}"
    );
    assert!(
        hover_text_b.contains("LspClient"),
        "Client B hover should mention LspClient, got: {hover_text_b}"
    );

    // ── 7. Both goto_definition on file_uri ─────────────────────────────
    let def_a = client_a
        .goto_definition(target_file_str, fn_line, fn_col)
        .await
        .expect("Client A: goto_definition failed");
    let def_b = client_b
        .goto_definition(target_file_str, fn_line, fn_col)
        .await
        .expect("Client B: goto_definition failed");

    // Both should get a result pointing to the same file and line
    assert!(def_a.is_some(), "Client A: goto_definition returned None");
    assert!(def_b.is_some(), "Client B: goto_definition returned None");

    // ── 8. Both find_references on LspClient ────────────────────────────
    let refs_a = client_a
        .find_references(target_file_str, struct_line, struct_col)
        .await
        .expect("Client A: find_references failed");
    let refs_b = client_b
        .find_references(target_file_str, struct_line, struct_col)
        .await
        .expect("Client B: find_references failed");

    let count_a = refs_a.as_ref().map_or(0, Vec::len);
    let count_b = refs_b.as_ref().map_or(0, Vec::len);

    assert!(
        count_a > 1,
        "Client A: expected multiple references to LspClient, got {count_a}"
    );
    assert!(
        count_b > 1,
        "Client B: expected multiple references to LspClient, got {count_b}"
    );

    // ── 9. Verify single rust-analyzer under our isolated lspmux server ─
    let server_pid = server_proc
        .id()
        .expect("lspmux server process should have a pid");
    let ra_children = count_direct_children_named(server_pid, "rust-analyzer");
    assert_eq!(
        ra_children, 1,
        "expected exactly one rust-analyzer child under lspmux server pid {server_pid}, found {ra_children}"
    );

    // ── 10. Shutdown ────────────────────────────────────────────────────
    client_a.shutdown().await;
    client_b.shutdown().await;
    let _ = server_proc.kill().await;
}

/// Two LSP clients with **different workspace roots** must see **different content**.
///
/// Proves the M1-gate question: when two worktrees (different paths, different
/// file contents) connect to the same lspmux daemon, each rust-analyzer instance
/// sees only its own files. Asserted via a marker symbol present in one
/// workspace and absent from the other.
#[tokio::test]
#[ignore = "requires lspmux + rust-analyzer binaries"]
#[allow(clippy::too_many_lines, clippy::similar_names)]
async fn two_worktrees_get_separate_rust_analyzers() {
    if !binary_exists("lspmux") || !binary_exists("rust-analyzer") {
        eprintln!("SKIP: lspmux and/or rust-analyzer not found on PATH");
        return;
    }
    let lspmux_bin = "lspmux";
    let ra_bin = "rust-analyzer";

    // ── Isolated lspmux daemon ──────────────────────────────────────────
    let tmp = tempfile::tempdir().expect("tempdir");
    let fake_home = tmp.path();
    let port = find_free_port();
    write_lspmux_config(fake_home, port);
    let home_str = fake_home.to_str().expect("home utf8");
    let env = [("HOME", home_str)];

    let mut server_proc = Command::new(lspmux_bin)
        .arg("server")
        .env("HOME", home_str)
        .kill_on_drop(true)
        .spawn()
        .expect("spawn lspmux server");
    assert!(wait_for_port(port, 10).await, "lspmux server didn't listen");

    // ── Two minimal Rust crates, each with a unique marker symbol ───────
    let wt_a = tmp.path().join("wt-a");
    let wt_b = tmp.path().join("wt-b");
    write_minimal_crate(&wt_a, "wt_a_marker", "MARKER_A_ONLY_SYMBOL");
    write_minimal_crate(&wt_b, "wt_b_marker", "MARKER_B_ONLY_SYMBOL");

    let wt_a_str = wt_a.to_str().expect("wt-a utf8");
    let wt_b_str = wt_b.to_str().expect("wt-b utf8");

    let client_a = LspClient::new_with_env(lspmux_bin, ra_bin, Some(wt_a_str), &env)
        .await
        .expect("client A");
    let client_b = LspClient::new_with_env(lspmux_bin, ra_bin, Some(wt_b_str), &env)
        .await
        .expect("client B");

    // Open each crate's lib.rs so rust-analyzer is forced to load the workspace.
    let lib_a = wt_a.join("src/lib.rs");
    let lib_b = wt_b.join("src/lib.rs");
    client_a
        .ensure_file_open(lib_a.to_str().unwrap())
        .await
        .expect("open A");
    client_b
        .ensure_file_open(lib_b.to_str().unwrap())
        .await
        .expect("open B");

    // Poll until both workspaces' markers are indexed, rather than guess at
    // an indexing budget. Two cold Cargo workspaces under a shared sccache +
    // nix-store toolchain typically settle within ~10s.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let a_sees_a = count_symbol_hits(&client_a, "MARKER_A_ONLY_SYMBOL").await;
        let b_sees_b = count_symbol_hits(&client_b, "MARKER_B_ONLY_SYMBOL").await;
        if a_sees_a >= 1 && b_sees_b >= 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "rust-analyzer did not index both workspaces within 30s"
        );
        sleep(Duration::from_millis(500)).await;
    }

    // ── Symmetric symbol probe ──────────────────────────────────────────
    let a_sees_a = count_symbol_hits(&client_a, "MARKER_A_ONLY_SYMBOL").await;
    let a_sees_b = count_symbol_hits(&client_a, "MARKER_B_ONLY_SYMBOL").await;
    let b_sees_a = count_symbol_hits(&client_b, "MARKER_A_ONLY_SYMBOL").await;
    let b_sees_b = count_symbol_hits(&client_b, "MARKER_B_ONLY_SYMBOL").await;

    assert!(
        a_sees_a >= 1,
        "workspace A should see its own marker, got {a_sees_a}"
    );
    assert!(
        b_sees_b >= 1,
        "workspace B should see its own marker, got {b_sees_b}"
    );
    assert_eq!(
        a_sees_b, 0,
        "workspace A must NOT see B's marker (cross-contamination), got {a_sees_b}"
    );
    assert_eq!(
        b_sees_a, 0,
        "workspace B must NOT see A's marker (cross-contamination), got {b_sees_a}"
    );

    // ── Daemon-side: two distinct rust-analyzer children ────────────────
    let server_pid = server_proc.id().expect("server pid");
    let ra_children = count_direct_children_named(server_pid, "rust-analyzer");
    assert_eq!(
        ra_children, 2,
        "expected two rust-analyzer children (one per worktree), found {ra_children}"
    );

    client_a.shutdown().await;
    client_b.shutdown().await;
    let _ = server_proc.kill().await;
}

/// Write a minimal Cargo crate at `root` with a single library function whose
/// name contains `marker`. The crate name is `pkg_name` so two crates can
/// coexist without name collisions in the global Cargo cache.
fn write_minimal_crate(root: &Path, pkg_name: &str, marker: &str) {
    std::fs::create_dir_all(root.join("src")).expect("create src dir");
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{pkg_name}"
version = "0.0.1"
edition = "2021"

[lib]
path = "src/lib.rs"
"#
        ),
    )
    .expect("write Cargo.toml");
    std::fs::write(
        root.join("src/lib.rs"),
        format!("pub fn {marker}() -> u32 {{ 42 }}\n"),
    )
    .expect("write lib.rs");
}

/// Issue a `workspace/symbol` query and return the number of matching results.
async fn count_symbol_hits(client: &LspClient, query: &str) -> usize {
    match client.workspace_symbols(query).await {
        Ok(Some(lsp_types::WorkspaceSymbolResponse::Flat(symbols))) => symbols
            .into_iter()
            .filter(|s| s.name.contains(query))
            .count(),
        Ok(Some(lsp_types::WorkspaceSymbolResponse::Nested(symbols))) => symbols
            .into_iter()
            .filter(|s| s.name.contains(query))
            .count(),
        _ => 0,
    }
}
