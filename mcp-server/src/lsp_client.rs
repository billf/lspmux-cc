//! LSP JSON-RPC client that communicates with lspmux via child process stdio.
//!
//! Spawns `lspmux client --server-path <ra>` and speaks LSP over its stdin/stdout.
//! Handles the `Content-Length` framing, request ID tracking, and the
//! `initialize`/`initialized` handshake.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use lsp_types::{
    request::{GotoDefinition, HoverRequest, References, Request},
    ClientCapabilities, DidChangeTextDocumentParams, DidOpenTextDocumentParams, InitializeParams,
    InitializedParams, TextDocumentContentChangeEvent, TextDocumentItem, Uri,
    VersionedTextDocumentIdentifier,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::time::{timeout, Duration};

/// A pending request awaiting its response.
type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>;

/// Timeout for LSP requests. Rust-analyzer can be slow on large workspaces,
/// but 30 seconds is generous enough for any single request.
const LSP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum allowed LSP message body size (100 MB). Prevents OOM from a
/// maliciously large `Content-Length` header.
const MAX_LSP_MESSAGE_SIZE: usize = 100 * 1024 * 1024;

/// LSP client that talks to lspmux/rust-analyzer via a child process.
pub struct LspClient {
    child_stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    next_id: AtomicI64,
    pending: PendingMap,
    /// Tracks files we've sent `didOpen` for: `(version, content_hash)`.
    /// The content hash is used to skip redundant `didChange` notifications.
    opened_files: Mutex<HashMap<String, (i32, u64)>>,
    child: Arc<Mutex<Child>>,
    /// Set to `false` when the reader task exits (child process died or stdout closed).
    alive: Arc<AtomicBool>,
}

/// Create a `file://` URI from an absolute file path.
///
/// # Errors
///
/// Returns an error if the path cannot be parsed as a valid URI.
pub fn file_uri(path: &str) -> Result<Uri> {
    if !std::path::Path::new(path).is_absolute() {
        bail!("invalid absolute file path for URI: {path}");
    }

    let uri_str = format!("file://{}", percent_encode_path(path));
    uri_str
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid file URI for path {path}: {e}"))
}

/// Extract a file path from a `file://` URI string.
pub fn uri_to_path(uri: &Uri) -> String {
    let s = uri.as_str();
    let path = s.strip_prefix("file://").unwrap_or(s);
    percent_decode_path(path).unwrap_or_else(|| path.to_string())
}

fn percent_encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        if is_unreserved_path_byte(b) {
            encoded.push(char::from(b));
        } else {
            encoded.push('%');
            encoded.push(hex_upper(b >> 4));
            encoded.push(hex_upper(b & 0x0f));
        }
    }
    encoded
}

fn percent_decode_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut i = 0;
    let mut decoded = Vec::with_capacity(bytes.len());
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_value(bytes[i + 1])?;
            let lo = hex_value(bytes[i + 2])?;
            decoded.push((hi << 4) | lo);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

const fn is_unreserved_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_' || b == b'~' || b == b'/'
}

const fn hex_upper(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + (nibble - 10)) as char,
        _ => '?',
    }
}

const fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Detect the LSP `languageId` from a file extension.
///
/// Falls back to `"plaintext"` for unrecognized extensions.
fn detect_language_id(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "jsx" => "javascriptreact",
        "tsx" => "typescriptreact",
        "c" => "c",
        "cpp" | "cc" | "cxx" | "h" | "hpp" => "cpp",
        "go" => "go",
        "rb" => "ruby",
        "sh" | "bash" | "zsh" => "shellscript",
        "css" => "css",
        "html" | "htm" => "html",
        "xml" => "xml",
        "sql" => "sql",
        "nix" => "nix",
        _ => "plaintext",
    }
}

impl LspClient {
    /// Spawn the lspmux client child process and perform the LSP handshake.
    ///
    /// # Errors
    ///
    /// Returns an error if the child process cannot be spawned or the LSP
    /// initialize handshake fails.
    pub async fn new(lspmux_bin: &str, ra_bin: &str, workspace_root: Option<&str>) -> Result<Self> {
        Self::new_with_env(lspmux_bin, ra_bin, workspace_root, &[]).await
    }

    /// Spawn the lspmux client with extra environment variables set on the child process.
    ///
    /// This is useful for integration tests that need an isolated lspmux instance
    /// (e.g. setting `HOME` to redirect the config file location).
    ///
    /// # Errors
    ///
    /// Returns an error if the child process cannot be spawned or the LSP
    /// initialize handshake fails.
    pub async fn new_with_env(
        lspmux_bin: &str,
        ra_bin: &str,
        workspace_root: Option<&str>,
        env: &[(&str, &str)],
    ) -> Result<Self> {
        let mut cmd = Command::new(lspmux_bin);
        cmd.arg("client")
            .arg("--server-path")
            .arg(ra_bin)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // Do not pipe stderr unless we actively drain it, otherwise verbose
            // child logging can fill the pipe buffer and block the process.
            .stderr(std::process::Stdio::inherit());
        for &(key, val) in env {
            cmd.env(key, val);
        }
        let mut child = cmd.spawn().context("failed to spawn lspmux client")?;

        let stdin = child.stdin.take().context("no stdin on child")?;
        let stdout = child.stdout.take().context("no stdout on child")?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let child_stdin = Arc::new(Mutex::new(stdin));
        let alive = Arc::new(AtomicBool::new(true));

        // Spawn reader task
        let pending_clone = Arc::clone(&pending);
        let alive_clone = Arc::clone(&alive);
        tokio::spawn(async move {
            let pending_for_cleanup = Arc::clone(&pending_clone);
            if let Err(e) = reader_loop(stdout, pending_clone).await {
                tracing::error!("LSP reader loop error: {e}");
            }
            // Signal that the child process is no longer responsive.
            alive_clone.store(false, Ordering::Release);
            // Drain pending requests so callers get immediate errors
            // (dropping senders causes RecvError on the corresponding receivers).
            let mut map = pending_for_cleanup.lock().await;
            let count = map.len();
            map.clear();
            drop(map);
            if count > 0 {
                tracing::warn!("Reader loop exited with {count} pending request(s)");
            }
        });

        let client = Self {
            child_stdin,
            next_id: AtomicI64::new(1),
            pending,
            opened_files: Mutex::new(HashMap::new()),
            child: Arc::new(Mutex::new(child)),
            alive,
        };

        // Initialize handshake
        let root_uri = workspace_root
            .map(file_uri)
            .transpose()
            .context("invalid workspace root URI")?;

        #[allow(deprecated)] // root_uri deprecated but still needed
        let init_params = InitializeParams {
            root_uri,
            capabilities: ClientCapabilities::default(),
            ..InitializeParams::default()
        };

        let _init_result = client
            .request::<lsp_types::request::Initialize>(init_params)
            .await
            .context("LSP initialize failed")?;

        // Send initialized notification
        client
            .notify("initialized", &InitializedParams {})
            .await
            .context("LSP initialized notification failed")?;

        tracing::info!("LSP client initialized");
        Ok(client)
    }

    /// Send a typed LSP request and await the response.
    ///
    /// # Errors
    ///
    /// Returns an error if the request times out, the server returns an error,
    /// or the response cannot be deserialized.
    pub async fn request<R: Request>(&self, params: R::Params) -> Result<R::Result>
    where
        R::Params: Serialize,
        R::Result: for<'de> Deserialize<'de>,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": R::METHOD,
            "params": serde_json::to_value(&params)?,
        });

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        if let Err(e) = self.send_message(&msg).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        let response = match timeout(LSP_REQUEST_TIMEOUT, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                self.pending.lock().await.remove(&id);
                bail!("LSP response channel closed (server may have crashed)");
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                bail!(
                    "LSP request timed out after {}s",
                    LSP_REQUEST_TIMEOUT.as_secs()
                );
            }
        };

        // Check for error
        if let Some(error) = response.get("error") {
            bail!("LSP error: {error}");
        }

        let result = response.get("result").cloned().unwrap_or(Value::Null);

        serde_json::from_value(result).context("failed to deserialize LSP response")
    }

    /// Send an LSP notification (no response expected).
    async fn notify<P: Serialize + Sync>(&self, method: &str, params: &P) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": serde_json::to_value(params)?,
        });
        self.send_message(&msg).await
    }

    /// Send a raw JSON-RPC message with `Content-Length` framing.
    ///
    /// Returns an error immediately if the child process is no longer alive.
    async fn send_message(&self, msg: &Value) -> Result<()> {
        if !self.alive.load(Ordering::Acquire) {
            bail!("LSP server is no longer running (child process exited)");
        }

        let body = serde_json::to_string(msg)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        let mut stdin = self.child_stdin.lock().await;
        stdin.write_all(header.as_bytes()).await?;
        stdin.write_all(body.as_bytes()).await?;
        stdin.flush().await?;
        drop(stdin);
        Ok(())
    }

    /// Send a `textDocument/hover` request.
    ///
    /// # Errors
    ///
    /// Returns an error if the LSP request fails.
    pub async fn hover(
        &self,
        file: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Hover>> {
        let params = lsp_types::HoverParams {
            text_document_position_params: text_doc_position(file, line, character)?,
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
        };
        self.request::<HoverRequest>(params).await
    }

    /// Send a `textDocument/definition` request.
    ///
    /// # Errors
    ///
    /// Returns an error if the LSP request fails.
    pub async fn goto_definition(
        &self,
        file: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::GotoDefinitionResponse>> {
        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: text_doc_position(file, line, character)?,
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        };
        self.request::<GotoDefinition>(params).await
    }

    /// Send a `textDocument/references` request.
    ///
    /// # Errors
    ///
    /// Returns an error if the LSP request fails.
    pub async fn find_references(
        &self,
        file: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<Vec<lsp_types::Location>>> {
        let params = lsp_types::ReferenceParams {
            text_document_position: text_doc_position(file, line, character)?,
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
        };
        self.request::<References>(params).await
    }

    /// Ensure a file is open in the LSP server with its current disk content.
    ///
    /// Sends `textDocument/didOpen` on first access, or `textDocument/didChange`
    /// with updated content on subsequent accesses. This is required by the LSP
    /// protocol before the server will provide diagnostics, hover, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read from disk or the notification
    /// fails to send.
    pub async fn ensure_file_open(&self, file_path: &str) -> Result<()> {
        let uri = file_uri(file_path)?;
        let content = tokio::fs::read_to_string(file_path)
            .await
            .with_context(|| format!("failed to read {file_path}"))?;

        let content_hash = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            content.hash(&mut hasher);
            hasher.finish()
        };

        let language_id = detect_language_id(file_path);

        let mut opened = self.opened_files.lock().await;
        if let Some((version, prev_hash)) = opened.get_mut(file_path) {
            if *prev_hash == content_hash {
                // File unchanged since last notification — skip didChange.
                return Ok(());
            }
            // Content changed — send didChange with updated content.
            *version += 1;
            *prev_hash = content_hash;
            let v = *version;
            drop(opened);

            self.notify(
                "textDocument/didChange",
                &DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier { uri, version: v },
                    content_changes: vec![TextDocumentContentChangeEvent {
                        range: None,
                        range_length: None,
                        text: content,
                    }],
                },
            )
            .await
        } else {
            // First access — send didOpen.
            opened.insert(file_path.to_string(), (0, content_hash));
            drop(opened);

            self.notify(
                "textDocument/didOpen",
                &DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri,
                        language_id: language_id.to_string(),
                        version: 0,
                        text: content,
                    },
                },
            )
            .await
        }
    }

    /// Gracefully shut down the LSP server and child process.
    ///
    /// Sends the LSP `shutdown` request, then `exit` notification, and finally
    /// kills the child process if it hasn't exited on its own.
    pub async fn shutdown(&self) {
        // Send LSP shutdown request (best-effort)
        if let Err(e) = self.request::<lsp_types::request::Shutdown>(()).await {
            tracing::warn!("LSP shutdown request failed: {e}");
        }

        // Send exit notification (best-effort)
        if let Err(e) = self.notify("exit", &()).await {
            tracing::warn!("LSP exit notification failed: {e}");
        }

        // Give the child a moment to exit, then kill it
        let mut child = self.child.lock().await;
        match timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => {
                tracing::info!("LSP child exited with {status}");
            }
            Ok(Err(e)) => {
                tracing::warn!("Error waiting for LSP child: {e}");
            }
            Err(_) => {
                tracing::warn!("LSP child did not exit in 5s, killing");
                if let Err(e) = child.kill().await {
                    tracing::error!("Failed to kill LSP child: {e}");
                }
            }
        }
    }
}

/// Build a `TextDocumentPositionParams` from a file path and position.
fn text_doc_position(
    file: &str,
    line: u32,
    character: u32,
) -> Result<lsp_types::TextDocumentPositionParams> {
    let uri = file_uri(file)?;
    Ok(lsp_types::TextDocumentPositionParams {
        text_document: lsp_types::TextDocumentIdentifier { uri },
        position: lsp_types::Position::new(line, character),
    })
}

/// Read LSP JSON-RPC messages from stdout and dispatch responses to pending requests.
async fn reader_loop(stdout: tokio::process::ChildStdout, pending: PendingMap) -> Result<()> {
    let mut reader = BufReader::new(stdout);

    loop {
        // Read headers until blank line
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                tracing::info!("LSP stdout closed");
                return Ok(());
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(len_str) = trimmed.strip_prefix("Content-Length: ") {
                content_length = Some(len_str.parse().context("invalid Content-Length")?);
            }
        }

        let length = content_length.context("missing Content-Length header")?;

        if length > MAX_LSP_MESSAGE_SIZE {
            bail!("LSP message size {length} exceeds maximum of {MAX_LSP_MESSAGE_SIZE}");
        }

        // Read body
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).await?;

        let msg: Value = serde_json::from_slice(&body).context("invalid JSON-RPC message")?;

        // If it has an id, it's a response to a request we sent
        if let Some(id) = msg.get("id").and_then(Value::as_i64) {
            let mut map = pending.lock().await;
            if let Some(tx) = map.remove(&id) {
                let _ = tx.send(msg);
            } else {
                tracing::warn!("received response for unknown request id {id}");
            }
        } else {
            // It's a notification from the server (e.g., diagnostics)
            let method = msg.get("method").and_then(Value::as_str).unwrap_or("?");
            tracing::debug!("LSP notification: {method}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_absolute_path() {
        let uri = file_uri("/tmp/test.rs").unwrap();
        assert_eq!(uri.as_str(), "file:///tmp/test.rs");
    }

    #[test]
    fn file_uri_percent_encodes_spaces() {
        let uri = file_uri("/tmp/space file.rs").unwrap();
        assert_eq!(uri.as_str(), "file:///tmp/space%20file.rs");
    }

    #[test]
    fn uri_to_path_round_trip() {
        let uri = file_uri("/tmp/test.rs").unwrap();
        assert_eq!(uri_to_path(&uri), "/tmp/test.rs");
    }

    #[test]
    fn uri_to_path_decodes_percent_encoding() {
        let uri: Uri = "file:///tmp/space%20file.rs".parse().unwrap();
        assert_eq!(uri_to_path(&uri), "/tmp/space file.rs");
    }

    #[test]
    fn detect_language_id_common_extensions() {
        assert_eq!(detect_language_id("/foo/bar.rs"), "rust");
        assert_eq!(detect_language_id("/foo/Cargo.toml"), "toml");
        assert_eq!(detect_language_id("/foo/bar.py"), "python");
        assert_eq!(detect_language_id("/foo/bar.ts"), "typescript");
        assert_eq!(detect_language_id("/foo/bar.tsx"), "typescriptreact");
        assert_eq!(detect_language_id("/foo/bar.go"), "go");
        assert_eq!(detect_language_id("/foo/bar.sh"), "shellscript");
        assert_eq!(detect_language_id("/foo/bar.nix"), "nix");
        assert_eq!(detect_language_id("/foo/bar.yml"), "yaml");
        assert_eq!(detect_language_id("/foo/bar.yaml"), "yaml");
        assert_eq!(detect_language_id("/foo/bar.json"), "json");
        assert_eq!(detect_language_id("/foo/bar.md"), "markdown");
        assert_eq!(detect_language_id("/foo/bar.html"), "html");
        assert_eq!(detect_language_id("/foo/bar.sql"), "sql");
        assert_eq!(detect_language_id("/foo/bar.unknown"), "plaintext");
        assert_eq!(detect_language_id("/foo/noext"), "plaintext");
    }

    #[test]
    fn text_doc_position_valid_path() {
        let params = text_doc_position("/tmp/test.rs", 10, 5).unwrap();
        assert_eq!(params.position.line, 10);
        assert_eq!(params.position.character, 5);
        assert!(params.text_document.uri.as_str().ends_with("/tmp/test.rs"));
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)]
    async fn request_send_failure_cleans_pending_entry() {
        let mut child = Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();

        let client = LspClient {
            child_stdin: Arc::new(Mutex::new(stdin)),
            next_id: AtomicI64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            opened_files: Mutex::new(HashMap::new()),
            child: Arc::new(Mutex::new(child)),
            alive: Arc::new(AtomicBool::new(false)),
        };

        let err = client.request::<lsp_types::request::Shutdown>(()).await;
        assert!(err.is_err());
        assert!(client.pending.lock().await.is_empty());

        {
            let mut child = client.child.lock().await;
            let _ = child.kill().await;
        }
    }
}
