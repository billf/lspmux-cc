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
    let uri_str = format!("file://{path}");
    uri_str
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid file URI for path {path}: {e}"))
}

/// Extract a file path from a `file://` URI string.
pub fn uri_to_path(uri: &Uri) -> String {
    let s = uri.as_str();
    s.strip_prefix("file://").unwrap_or(s).to_string()
}

impl LspClient {
    /// Spawn the lspmux client child process and perform the LSP handshake.
    pub async fn new(lspmux_bin: &str, ra_bin: &str, workspace_root: Option<&str>) -> Result<Self> {
        let mut child = Command::new(lspmux_bin)
            .arg("client")
            .arg("--server-path")
            .arg(ra_bin)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("failed to spawn lspmux client")?;

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

        self.send_message(&msg).await?;

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

        let language_id = if std::path::Path::new(file_path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        {
            "rust"
        } else {
            "toml"
        };

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
    fn uri_to_path_round_trip() {
        let uri = file_uri("/tmp/test.rs").unwrap();
        assert_eq!(uri_to_path(&uri), "/tmp/test.rs");
    }

    #[test]
    fn text_doc_position_valid_path() {
        let params = text_doc_position("/tmp/test.rs", 10, 5).unwrap();
        assert_eq!(params.position.line, 10);
        assert_eq!(params.position.character, 5);
        assert!(params.text_document.uri.as_str().ends_with("/tmp/test.rs"));
    }
}
