//! MCP tool definitions for rust-analyzer access via lspmux.
//!
//! Four read-only tools:
//! - `rust_diagnostics`: Get errors/warnings for a file
//! - `rust_hover`: Get type signature + docs at a position
//! - `rust_goto_definition`: Find definition location
//! - `rust_find_references`: Find all references

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ListToolsResult};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_router, ErrorData as McpError, RoleServer};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::lsp_client::{uri_to_path, LspClient};

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

/// Format a location as `file:line:col`.
fn format_location(loc: &lsp_types::Location) -> String {
    let path = uri_to_path(&loc.uri);
    format!(
        "{}:{}:{}",
        path,
        loc.range.start.line + 1,
        loc.range.start.character + 1,
    )
}

/// MCP server providing rust-analyzer tools via lspmux.
#[derive(Clone)]
pub struct RustAnalyzerTools {
    lsp: Arc<LspClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl RustAnalyzerTools {
    /// Create a new tools instance wrapping an LSP client.
    pub fn new(lsp: Arc<LspClient>) -> Self {
        Self {
            lsp,
            tool_router: Self::tool_router(),
        }
    }

    /// Get diagnostics (errors and warnings) for a Rust file.
    #[tool(
        name = "rust_diagnostics",
        description = "Get Rust compiler errors and warnings for a file. Returns diagnostics with line numbers, severity, and messages."
    )]
    async fn diagnostics(&self, params: Parameters<FileParam>) -> Result<CallToolResult, McpError> {
        let file = &params.0.file_path;

        let uri: lsp_types::Uri = format!("file://{file}")
            .parse()
            .map_err(|e| McpError::invalid_params(format!("invalid file path: {e}"), None))?;

        let diag_params = lsp_types::DocumentDiagnosticParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            identifier: None,
            previous_result_id: None,
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        };

        match self
            .lsp
            .request::<lsp_types::request::DocumentDiagnosticRequest>(diag_params)
            .await
        {
            Ok(report) => {
                let items = match report {
                    lsp_types::DocumentDiagnosticReportResult::Report(
                        lsp_types::DocumentDiagnosticReport::Full(full),
                    ) => full.full_document_diagnostic_report.items,
                    lsp_types::DocumentDiagnosticReportResult::Report(
                        lsp_types::DocumentDiagnosticReport::Unchanged(_),
                    )
                    | lsp_types::DocumentDiagnosticReportResult::Partial(_) => vec![],
                };

                if items.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "No diagnostics found.",
                    )]));
                }

                let text = items
                    .iter()
                    .map(|d| {
                        let severity = match d.severity {
                            Some(lsp_types::DiagnosticSeverity::ERROR) => "ERROR",
                            Some(lsp_types::DiagnosticSeverity::WARNING) => "WARNING",
                            Some(lsp_types::DiagnosticSeverity::INFORMATION) => "INFO",
                            Some(lsp_types::DiagnosticSeverity::HINT) => "HINT",
                            _ => "UNKNOWN",
                        };
                        format!(
                            "{}:{}: [{}] {}",
                            d.range.start.line + 1,
                            d.range.start.character + 1,
                            severity,
                            d.message,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Diagnostics request failed: {e}\n\n\
                 Note: rust-analyzer may still be loading. Try again in a few seconds."
            ))])),
        }
    }

    /// Get type information and documentation at a position.
    #[tool(
        name = "rust_hover",
        description = "Get type signature and documentation for a symbol at a specific position in a Rust file."
    )]
    async fn hover(&self, params: Parameters<PositionParam>) -> Result<CallToolResult, McpError> {
        let p = &params.0;
        match self.lsp.hover(&p.file_path, p.line, p.character).await {
            Ok(Some(hover)) => {
                let text = match hover.contents {
                    lsp_types::HoverContents::Markup(markup) => markup.value,
                    lsp_types::HoverContents::Scalar(lsp_types::MarkedString::String(s)) => s,
                    lsp_types::HoverContents::Scalar(lsp_types::MarkedString::LanguageString(
                        ls,
                    )) => format!("```{}\n{}\n```", ls.language, ls.value),
                    lsp_types::HoverContents::Array(items) => items
                        .into_iter()
                        .map(|item| match item {
                            lsp_types::MarkedString::String(s) => s,
                            lsp_types::MarkedString::LanguageString(ls) => {
                                format!("```{}\n{}\n```", ls.language, ls.value)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                };
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Ok(None) => Ok(CallToolResult::success(vec![Content::text(
                "No hover information available at this position.",
            )])),
            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Hover request failed: {e}"
            ))])),
        }
    }

    /// Find the definition of a symbol.
    #[tool(
        name = "rust_goto_definition",
        description = "Find where a symbol is defined. Returns the file path and line number of the definition."
    )]
    async fn goto_definition(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<CallToolResult, McpError> {
        let p = &params.0;
        match self
            .lsp
            .goto_definition(&p.file_path, p.line, p.character)
            .await
        {
            Ok(Some(response)) => {
                let locations = match response {
                    lsp_types::GotoDefinitionResponse::Scalar(loc) => vec![loc],
                    lsp_types::GotoDefinitionResponse::Array(locs) => locs,
                    lsp_types::GotoDefinitionResponse::Link(links) => links
                        .into_iter()
                        .map(|link| lsp_types::Location {
                            uri: link.target_uri,
                            range: link.target_selection_range,
                        })
                        .collect(),
                };

                if locations.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "No definition found.",
                    )]));
                }

                let text = locations
                    .iter()
                    .map(format_location)
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Ok(None) => Ok(CallToolResult::success(vec![Content::text(
                "No definition found at this position.",
            )])),
            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Go to definition failed: {e}"
            ))])),
        }
    }

    /// Find all references to a symbol.
    #[tool(
        name = "rust_find_references",
        description = "Find all references to a symbol at a specific position. Returns a list of file paths and line numbers."
    )]
    async fn find_references(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<CallToolResult, McpError> {
        let p = &params.0;
        match self
            .lsp
            .find_references(&p.file_path, p.line, p.character)
            .await
        {
            Ok(Some(locations)) => {
                if locations.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "No references found.",
                    )]));
                }

                let text = locations
                    .iter()
                    .map(format_location)
                    .collect::<Vec<_>>()
                    .join("\n");
                let header = format!("Found {} reference(s):\n", locations.len());
                Ok(CallToolResult::success(vec![Content::text(header + &text)]))
            }
            Ok(None) => Ok(CallToolResult::success(vec![Content::text(
                "No references found at this position.",
            )])),
            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Find references failed: {e}"
            ))])),
        }
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
