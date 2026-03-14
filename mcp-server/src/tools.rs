//! MCP tool definitions for rust-analyzer access via lspmux.
//!
//! Six read-only tools:
//! - `rust_diagnostics`: Get errors/warnings for a file
//! - `rust_hover`: Get type signature + docs at a position
//! - `rust_goto_definition`: Find definition location
//! - `rust_find_references`: Find all references
//! - `rust_workspace_symbol`: Search symbols by name across the workspace
//! - `rust_server_status`: Check server health and workspace bootstrap status

use std::path::Path;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolRequestParams, CallToolResult, ListToolsResult};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_router, ErrorData as McpError, Json, RoleServer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use lspmux_cc_mcp::bootstrap::{RuntimeStatus, SERVER_NAME};
use lspmux_cc_mcp::lsp_client::{file_uri, uri_to_path, LspClient};

/// Validate that a file path is absolute and exists on disk.
///
/// Returns an `McpError::invalid_params` if the path is relative or does not exist.
fn validate_file_path(path: &str) -> Result<(), McpError> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(McpError::invalid_params(
            format!("file_path must be absolute, got: {path}"),
            None,
        ));
    }
    if !p.exists() {
        return Err(McpError::invalid_params(
            format!("file not found: {path}"),
            None,
        ));
    }
    Ok(())
}

fn internal_error(msg: impl Into<String>) -> McpError {
    McpError::internal_error(msg.into(), None)
}

fn diagnostic_severity_name(severity: Option<lsp_types::DiagnosticSeverity>) -> &'static str {
    match severity {
        Some(lsp_types::DiagnosticSeverity::ERROR) => "error",
        Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
        Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
        Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
        _ => "unknown",
    }
}

fn symbol_kind_name(kind: lsp_types::SymbolKind) -> &'static str {
    match kind {
        lsp_types::SymbolKind::FILE => "file",
        lsp_types::SymbolKind::MODULE => "module",
        lsp_types::SymbolKind::NAMESPACE => "namespace",
        lsp_types::SymbolKind::PACKAGE => "package",
        lsp_types::SymbolKind::CLASS => "class",
        lsp_types::SymbolKind::METHOD => "method",
        lsp_types::SymbolKind::PROPERTY => "property",
        lsp_types::SymbolKind::FIELD => "field",
        lsp_types::SymbolKind::CONSTRUCTOR => "constructor",
        lsp_types::SymbolKind::ENUM => "enum",
        lsp_types::SymbolKind::INTERFACE => "interface",
        lsp_types::SymbolKind::FUNCTION => "function",
        lsp_types::SymbolKind::VARIABLE => "variable",
        lsp_types::SymbolKind::CONSTANT => "constant",
        lsp_types::SymbolKind::STRING => "string",
        lsp_types::SymbolKind::NUMBER => "number",
        lsp_types::SymbolKind::BOOLEAN => "boolean",
        lsp_types::SymbolKind::ARRAY => "array",
        lsp_types::SymbolKind::OBJECT => "object",
        lsp_types::SymbolKind::KEY => "key",
        lsp_types::SymbolKind::NULL => "null",
        lsp_types::SymbolKind::ENUM_MEMBER => "enum_member",
        lsp_types::SymbolKind::STRUCT => "struct",
        lsp_types::SymbolKind::EVENT => "event",
        lsp_types::SymbolKind::OPERATOR => "operator",
        lsp_types::SymbolKind::TYPE_PARAMETER => "type_parameter",
        _ => "unknown",
    }
}

fn markup_to_text(contents: lsp_types::HoverContents) -> String {
    match contents {
        lsp_types::HoverContents::Markup(markup) => markup.value,
        lsp_types::HoverContents::Scalar(lsp_types::MarkedString::String(value)) => value,
        lsp_types::HoverContents::Scalar(lsp_types::MarkedString::LanguageString(value)) => {
            format!("```{}\n{}\n```", value.language, value.value)
        }
        lsp_types::HoverContents::Array(items) => items
            .into_iter()
            .map(|item| match item {
                lsp_types::MarkedString::String(value) => value,
                lsp_types::MarkedString::LanguageString(value) => {
                    format!("```{}\n{}\n```", value.language, value.value)
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

/// Tool parameter: a file path.
#[derive(Deserialize, JsonSchema)]
pub struct FileParam {
    /// Absolute path to the Rust source file.
    pub file_path: String,
}

/// Tool parameters: file path + position (line, character).
#[derive(Deserialize, JsonSchema)]
pub struct PositionParam {
    /// Absolute path to the Rust source file.
    pub file_path: String,
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based character offset.
    pub character: u32,
}

/// Tool parameters: workspace symbol search query.
#[derive(Deserialize, JsonSchema)]
pub struct WorkspaceSymbolParam {
    /// Substring to search for in symbol names across the workspace.
    pub query: String,
}

/// Empty parameter struct for tools that take no arguments.
#[derive(Deserialize, JsonSchema)]
pub struct NoParams {}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct LocationRecord {
    pub file_path: String,
    pub uri: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub display: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct PositionRecord {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct RangeRecord {
    pub start: PositionRecord,
    pub end: PositionRecord,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DiagnosticRecord {
    pub severity: String,
    pub message: String,
    pub code: Option<String>,
    pub source: Option<String>,
    pub location: LocationRecord,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DiagnosticsResponse {
    pub file_path: String,
    pub diagnostic_count: usize,
    pub diagnostics: Vec<DiagnosticRecord>,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct HoverResponse {
    pub file_path: String,
    pub requested_position: PositionRecord,
    pub found: bool,
    pub contents: String,
    pub range: Option<RangeRecord>,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct LocationsResponse {
    pub file_path: String,
    pub requested_position: PositionRecord,
    pub found: bool,
    pub location_count: usize,
    pub locations: Vec<LocationRecord>,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceSymbolRecord {
    pub name: String,
    pub kind: String,
    pub container_name: Option<String>,
    pub location: LocationRecord,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceSymbolsResponse {
    pub query: String,
    pub symbol_count: usize,
    pub symbols: Vec<WorkspaceSymbolRecord>,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ServerStatusResponse {
    pub server: String,
    pub server_status: String,
    pub workspace_root: Option<String>,
    pub server_version: Option<String>,
    pub runtime: RuntimeStatus,
    pub summary: String,
}

fn location_record(uri: &lsp_types::Uri, range: &lsp_types::Range) -> LocationRecord {
    let file_path = uri_to_path(uri);
    LocationRecord {
        display: format!(
            "{}:{}:{}",
            file_path,
            range.start.line + 1,
            range.start.character + 1,
        ),
        file_path,
        uri: uri.to_string(),
        line: range.start.line + 1,
        column: range.start.character + 1,
        end_line: range.end.line + 1,
        end_column: range.end.character + 1,
    }
}

fn range_record(range: &lsp_types::Range) -> RangeRecord {
    RangeRecord {
        start: PositionRecord {
            line: range.start.line + 1,
            character: range.start.character + 1,
        },
        end: PositionRecord {
            line: range.end.line + 1,
            character: range.end.character + 1,
        },
    }
}

/// MCP server providing rust-analyzer tools via lspmux.
#[derive(Clone)]
pub struct RustAnalyzerTools {
    lsp: Arc<LspClient>,
    runtime_status: RuntimeStatus,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl RustAnalyzerTools {
    /// Create a new tools instance wrapping an LSP client.
    pub fn new(lsp: Arc<LspClient>, runtime_status: RuntimeStatus) -> Self {
        Self {
            lsp,
            runtime_status,
            tool_router: Self::tool_router(),
        }
    }

    /// Get diagnostics (errors and warnings) for a Rust file.
    #[tool(
        name = "rust_diagnostics",
        description = "Get Rust compiler errors and warnings for a file. Returns structured diagnostics with one-based locations."
    )]
    async fn diagnostics(
        &self,
        params: Parameters<FileParam>,
    ) -> Result<Json<DiagnosticsResponse>, McpError> {
        let file = &params.0.file_path;
        validate_file_path(file)?;

        self.lsp
            .ensure_file_open(file)
            .await
            .map_err(|e| internal_error(format!("failed to synchronize file with lspmux: {e}")))?;

        let uri = file_uri(file)
            .map_err(|e| McpError::invalid_params(format!("invalid file path: {e}"), None))?;

        let diagnostic_uri = uri.clone();
        let diag_params = lsp_types::DocumentDiagnosticParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            identifier: None,
            previous_result_id: None,
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        };

        let report = self
            .lsp
            .request::<lsp_types::request::DocumentDiagnosticRequest>(diag_params)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "diagnostics request failed: {e}. rust-analyzer may still be indexing"
                ))
            })?;

        let items = match report {
            lsp_types::DocumentDiagnosticReportResult::Report(
                lsp_types::DocumentDiagnosticReport::Full(full),
            ) => full.full_document_diagnostic_report.items,
            lsp_types::DocumentDiagnosticReportResult::Report(
                lsp_types::DocumentDiagnosticReport::Unchanged(_),
            )
            | lsp_types::DocumentDiagnosticReportResult::Partial(_) => vec![],
        };

        let diagnostics = items
            .into_iter()
            .map(|diagnostic| DiagnosticRecord {
                severity: diagnostic_severity_name(diagnostic.severity).to_string(),
                message: diagnostic.message,
                code: diagnostic.code.map(|code| match code {
                    lsp_types::NumberOrString::String(value) => value,
                    lsp_types::NumberOrString::Number(value) => value.to_string(),
                }),
                source: diagnostic.source,
                location: location_record(&diagnostic_uri, &diagnostic.range),
            })
            .collect::<Vec<_>>();

        let diagnostic_count = diagnostics.len();
        let summary = if diagnostic_count == 0 {
            format!("No diagnostics found for {file}.")
        } else {
            format!("Found {diagnostic_count} diagnostic(s) for {file}.")
        };

        Ok(Json(DiagnosticsResponse {
            file_path: file.clone(),
            diagnostic_count,
            diagnostics,
            summary,
        }))
    }

    /// Get type information and documentation at a position.
    #[tool(
        name = "rust_hover",
        description = "Get type signature and documentation for a symbol at a specific position in a Rust file."
    )]
    async fn hover(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<Json<HoverResponse>, McpError> {
        let p = &params.0;
        validate_file_path(&p.file_path)?;

        self.lsp
            .ensure_file_open(&p.file_path)
            .await
            .map_err(|e| internal_error(format!("failed to synchronize file with lspmux: {e}")))?;

        let requested_position = PositionRecord {
            line: p.line,
            character: p.character,
        };
        let hover = self
            .lsp
            .hover(&p.file_path, p.line, p.character)
            .await
            .map_err(|e| internal_error(format!("hover request failed: {e}")))?;

        match hover {
            Some(hover) => {
                let contents = markup_to_text(hover.contents);
                Ok(Json(HoverResponse {
                    file_path: p.file_path.clone(),
                    requested_position,
                    found: true,
                    range: hover.range.as_ref().map(range_record),
                    summary: format!("Hover information found for {}.", p.file_path),
                    contents,
                }))
            }
            None => Ok(Json(HoverResponse {
                file_path: p.file_path.clone(),
                requested_position,
                found: false,
                contents: String::new(),
                range: None,
                summary: "No hover information available at this position.".to_string(),
            })),
        }
    }

    /// Find the definition of a symbol.
    #[tool(
        name = "rust_goto_definition",
        description = "Find where a symbol is defined. Returns one-based file locations for the definition."
    )]
    async fn goto_definition(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<Json<LocationsResponse>, McpError> {
        let p = &params.0;
        validate_file_path(&p.file_path)?;

        self.lsp
            .ensure_file_open(&p.file_path)
            .await
            .map_err(|e| internal_error(format!("failed to synchronize file with lspmux: {e}")))?;

        let response = self
            .lsp
            .goto_definition(&p.file_path, p.line, p.character)
            .await
            .map_err(|e| internal_error(format!("go to definition failed: {e}")))?;

        let locations = match response {
            Some(lsp_types::GotoDefinitionResponse::Scalar(location)) => {
                vec![location_record(&location.uri, &location.range)]
            }
            Some(lsp_types::GotoDefinitionResponse::Array(locations)) => locations
                .into_iter()
                .map(|location| location_record(&location.uri, &location.range))
                .collect(),
            Some(lsp_types::GotoDefinitionResponse::Link(links)) => links
                .into_iter()
                .map(|link| location_record(&link.target_uri, &link.target_selection_range))
                .collect(),
            None => vec![],
        };

        let found = !locations.is_empty();
        let location_count = locations.len();
        let summary = if found {
            format!("Found {location_count} definition location(s).")
        } else {
            "No definition found at this position.".to_string()
        };

        Ok(Json(LocationsResponse {
            file_path: p.file_path.clone(),
            requested_position: PositionRecord {
                line: p.line,
                character: p.character,
            },
            found,
            location_count,
            locations,
            summary,
        }))
    }

    /// Find all references to a symbol.
    #[tool(
        name = "rust_find_references",
        description = "Find all references to a symbol at a specific position. Returns one-based file locations."
    )]
    async fn find_references(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<Json<LocationsResponse>, McpError> {
        let p = &params.0;
        validate_file_path(&p.file_path)?;

        self.lsp
            .ensure_file_open(&p.file_path)
            .await
            .map_err(|e| internal_error(format!("failed to synchronize file with lspmux: {e}")))?;

        let locations = self
            .lsp
            .find_references(&p.file_path, p.line, p.character)
            .await
            .map_err(|e| internal_error(format!("find references failed: {e}")))?
            .unwrap_or_default()
            .into_iter()
            .map(|location| location_record(&location.uri, &location.range))
            .collect::<Vec<_>>();

        let found = !locations.is_empty();
        let location_count = locations.len();
        let summary = if found {
            format!("Found {location_count} reference(s).")
        } else {
            "No references found at this position.".to_string()
        };

        Ok(Json(LocationsResponse {
            file_path: p.file_path.clone(),
            requested_position: PositionRecord {
                line: p.line,
                character: p.character,
            },
            found,
            location_count,
            locations,
            summary,
        }))
    }

    /// Search for symbols by name across the workspace.
    #[tool(
        name = "rust_workspace_symbol",
        description = "Search for symbols by name across the entire workspace. Returns one-based locations and normalized symbol kinds."
    )]
    async fn workspace_symbol(
        &self,
        params: Parameters<WorkspaceSymbolParam>,
    ) -> Result<Json<WorkspaceSymbolsResponse>, McpError> {
        let query = &params.0.query;
        let symbols = self
            .lsp
            .workspace_symbols(query.clone())
            .await
            .map_err(|e| internal_error(format!("workspace symbol search failed: {e}")))?;

        let records = match symbols {
            Some(lsp_types::WorkspaceSymbolResponse::Flat(symbols)) => symbols
                .into_iter()
                .map(|symbol| WorkspaceSymbolRecord {
                    name: symbol.name,
                    kind: symbol_kind_name(symbol.kind).to_string(),
                    container_name: symbol.container_name,
                    location: location_record(&symbol.location.uri, &symbol.location.range),
                })
                .collect(),
            Some(lsp_types::WorkspaceSymbolResponse::Nested(symbols)) => symbols
                .into_iter()
                .filter_map(|symbol| {
                    if let lsp_types::OneOf::Left(location) = symbol.location {
                        Some(WorkspaceSymbolRecord {
                            name: symbol.name,
                            kind: symbol_kind_name(symbol.kind).to_string(),
                            container_name: symbol.container_name,
                            location: location_record(&location.uri, &location.range),
                        })
                    } else {
                        None
                    }
                })
                .collect(),
            None => vec![],
        };

        let symbol_count = records.len();
        let summary = if symbol_count == 0 {
            format!("No symbols found matching {query:?}.")
        } else {
            format!("Found {symbol_count} symbol(s) matching {query:?}.")
        };

        Ok(Json(WorkspaceSymbolsResponse {
            query: query.clone(),
            symbol_count,
            symbols: records,
            summary,
        }))
    }

    /// Return server health and configuration status.
    #[tool(
        name = "rust_server_status",
        description = "Check rust-analyzer server health, active workspace root, and shared lspmux bootstrap metadata."
    )]
    async fn server_status(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<ServerStatusResponse>, McpError> {
        let server_status = if self.lsp.is_alive() {
            "running"
        } else {
            "stopped"
        };
        let workspace_root = self.lsp.workspace_root().await;
        let server_version = self.lsp.server_version().await;
        let summary = format!(
            "{SERVER_NAME} is {server_status}; workspace root: {}",
            workspace_root
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string())
        );

        Ok(Json(ServerStatusResponse {
            server: SERVER_NAME.to_string(),
            server_status: server_status.to_string(),
            workspace_root,
            server_version,
            runtime: self.runtime_status.clone(),
            summary,
        }))
    }
}

/// Delegation methods for `ServerHandler` integration.
impl RustAnalyzerTools {
    /// List all available tools.
    pub fn list_tools(&self) -> ListToolsResult {
        ListToolsResult {
            tools: self.tool_router.list_all(),
            ..ListToolsResult::default()
        }
    }

    /// Call a tool by name.
    pub async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let ctx = ToolCallContext::new(self, request, context);
        self.tool_router.call(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_file_path_rejects_relative() {
        let err = validate_file_path("relative/path.rs").unwrap_err();
        assert!(err.message.contains("must be absolute"));
    }

    #[test]
    fn validate_file_path_rejects_nonexistent() {
        let err = validate_file_path("/nonexistent/path/to/file.rs").unwrap_err();
        assert!(err.message.contains("file not found"));
    }

    #[test]
    fn validate_file_path_accepts_existing_absolute() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = format!("{manifest}/Cargo.toml");
        assert!(validate_file_path(&path).is_ok());
    }

    #[test]
    fn workspace_symbol_param_deserializes() {
        let json = serde_json::json!({ "query": "MyStruct" });
        let param: WorkspaceSymbolParam = serde_json::from_value(json).unwrap();
        assert_eq!(param.query, "MyStruct");
    }

    #[test]
    fn no_params_deserializes_from_empty_object() {
        let json = serde_json::json!({});
        let _param: NoParams = serde_json::from_value(json).unwrap();
    }

    #[test]
    fn location_record_is_one_based() {
        let loc = lsp_types::Location {
            uri: lspmux_cc_mcp::lsp_client::file_uri("/tmp/test.rs").unwrap(),
            range: lsp_types::Range {
                start: lsp_types::Position::new(0, 0),
                end: lsp_types::Position::new(0, 5),
            },
        };
        let formatted = location_record(&loc.uri, &loc.range);
        assert_eq!(formatted.display, "/tmp/test.rs:1:1");
        assert_eq!(formatted.line, 1);
        assert_eq!(formatted.column, 1);
    }

    #[test]
    fn range_record_is_one_based() {
        let range = lsp_types::Range {
            start: lsp_types::Position::new(0, 1),
            end: lsp_types::Position::new(2, 3),
        };
        let formatted = range_record(&range);
        assert_eq!(formatted.start.line, 1);
        assert_eq!(formatted.start.character, 2);
        assert_eq!(formatted.end.line, 3);
        assert_eq!(formatted.end.character, 4);
    }

    #[test]
    fn markup_to_text_preserves_language_blocks() {
        let text = markup_to_text(lsp_types::HoverContents::Scalar(
            lsp_types::MarkedString::LanguageString(lsp_types::LanguageString {
                language: "rust".to_string(),
                value: "fn demo()".to_string(),
            }),
        ));
        assert!(text.contains("```rust"));
        assert!(text.contains("fn demo()"));
    }
}
