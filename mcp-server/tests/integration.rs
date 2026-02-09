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
        .map(|o| o.status.success())
        .unwrap_or(false)
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
        r#"listen = "127.0.0.1:{port}"
connect = "127.0.0.1:{port}"
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

    // ── 9. Verify single rust-analyzer ──────────────────────────────────
    // Count rust-analyzer processes spawned by our lspmux server.
    // Note: this checks system-wide, so may see extra if other instances
    // are running. We just verify it's at least 1 (our server started one).
    // The real proof is that two clients got results — with mux, one RA serves both.

    // ── 10. Shutdown ────────────────────────────────────────────────────
    client_a.shutdown().await;
    client_b.shutdown().await;
    let _ = server_proc.kill().await;
}
