//! MCP tool definitions for rust-analyzer access via lspmux.
//!
//! Read-only tools: every tool returns data and never mutates the user's files;
//! edits from `rust_code_actions` and `rust_rename` are returned for the agent
//! to apply itself.
//!
//! - `rust_diagnostics`: Get errors/warnings for a file
//! - `rust_hover`: Get type signature + docs at a position
//! - `rust_goto_definition`: Find definition location
//! - `rust_goto_implementation`: Find trait/method implementations
//! - `rust_find_references`: Find all references
//! - `rust_workspace_symbol`: Search symbols by name across the workspace
//! - `rust_document_symbols`: Outline a file's symbol tree
//! - `rust_code_actions`: List quick fixes / refactors (with their edits) for a range
//! - `rust_rename`: Compute the rename workspace edit (unapplied)
//! - `rust_call_hierarchy_incoming` / `rust_call_hierarchy_outgoing`: Callers / callees
//! - `rust_expand_macro`: Expand a macro invocation
//! - `rust_server_status`: Check server health, readiness, and workspace bootstrap status
//! - `rust_workspace_registry`: List all daemon-hosted rust-analyzer instances

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolRequestParams, CallToolResult, ErrorCode, ListToolsResult};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_router, ErrorData as McpError, Json, RoleServer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use lspmux_cc_mcp::bootstrap::{InstanceRecord, RuntimeStatus, SERVER_NAME};
use lspmux_cc_mcp::lsp_client::{file_uri, uri_to_path, LspClient};
use lspmux_cc_mcp::telemetry::{
    ClientIdentity, CompilerAccountingSnapshot, ReadinessState, TelemetrySnapshot, TelemetryState,
    ToolOutcome,
};

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

const fn diagnostic_severity_name(severity: Option<lsp_types::DiagnosticSeverity>) -> &'static str {
    match severity {
        Some(lsp_types::DiagnosticSeverity::ERROR) => "error",
        Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
        Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
        Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
        _ => "unknown",
    }
}

const fn symbol_kind_name(kind: lsp_types::SymbolKind) -> &'static str {
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

/// Tool parameters: file path + a zero-based range (start/end line+character).
#[derive(Deserialize, JsonSchema)]
pub struct RangeParam {
    /// Absolute path to the Rust source file.
    pub file_path: String,
    /// Zero-based start line.
    pub start_line: u32,
    /// Zero-based start character offset.
    pub start_character: u32,
    /// Zero-based end line.
    pub end_line: u32,
    /// Zero-based end character offset.
    pub end_character: u32,
}

/// Tool parameters: file path + position + the new symbol name.
#[derive(Deserialize, JsonSchema)]
pub struct RenameParam {
    /// Absolute path to the Rust source file.
    pub file_path: String,
    /// Zero-based line number of the symbol to rename.
    pub line: u32,
    /// Zero-based character offset of the symbol to rename.
    pub character: u32,
    /// The new name for the symbol.
    pub new_name: String,
}

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
    pub client: ClientIdentity,
    pub readiness: ReadinessState,
    pub telemetry: TelemetrySnapshot,
    pub compiler_accounting: CompilerAccountingSnapshot,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct RegistrySnapshot {
    /// True if `lspmux status --json` produced parseable output.
    pub daemon_reachable: bool,
    /// All rust-analyzer instances the lspmux daemon is currently hosting.
    pub instances: Vec<InstanceRecord>,
    /// Number of rust-analyzer instances the daemon is hosting. Zero is
    /// meaningful only when `daemon_reachable` is true.
    pub instance_count: usize,
    pub summary: String,
}

/// A single text edit (range + replacement text), one-based.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TextEditRecord {
    pub range: RangeRecord,
    pub new_text: String,
}

/// All edits a workspace edit makes to one file.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct FileEditRecord {
    pub file_path: String,
    pub uri: String,
    pub edits: Vec<TextEditRecord>,
}

/// A workspace edit shaped as data: per-file text edits, never applied.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub struct WorkspaceEditRecord {
    pub file_count: usize,
    pub edit_count: usize,
    pub files: Vec<FileEditRecord>,
}

/// A single code action: its title, kind, the edit it would make, and/or the
/// command it would run. The edit is data only; it is not applied.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CodeActionRecord {
    pub title: String,
    pub kind: Option<String>,
    pub is_preferred: bool,
    /// The command identifier, when the action runs a command instead of (or in
    /// addition to) applying an edit.
    pub command: Option<String>,
    /// Arguments the command handler expects, preserved so an agent can reproduce
    /// the command invocation. `None` for edit-only actions.
    pub command_arguments: Option<Vec<serde_json::Value>>,
    /// The workspace edit the action would apply, when it carries one.
    pub edit: Option<WorkspaceEditRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CodeActionsResponse {
    pub file_path: String,
    pub requested_range: RangeRecord,
    pub action_count: usize,
    pub actions: Vec<CodeActionRecord>,
    pub summary: String,
}

/// A node in a file's symbol tree. `children` is empty for flat
/// (`SymbolInformation`) responses.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentSymbolRecord {
    pub name: String,
    pub detail: Option<String>,
    pub kind: String,
    pub range: RangeRecord,
    pub selection_range: RangeRecord,
    pub children: Vec<Self>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentSymbolsResponse {
    pub file_path: String,
    pub symbol_count: usize,
    pub symbols: Vec<DocumentSymbolRecord>,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct RenameResponse {
    pub file_path: String,
    pub requested_position: PositionRecord,
    pub new_name: String,
    pub found: bool,
    pub edit: Option<WorkspaceEditRecord>,
    pub summary: String,
}

/// A call hierarchy participant (a caller or callee).
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CallHierarchyItemRecord {
    pub name: String,
    pub kind: String,
    pub detail: Option<String>,
    pub location: LocationRecord,
}

/// One incoming or outgoing call: the other participant plus the call-site
/// ranges within the queried symbol.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CallHierarchyCallRecord {
    pub item: CallHierarchyItemRecord,
    pub from_ranges: Vec<RangeRecord>,
}

/// Which way a call hierarchy query points. Serializes as `"incoming"` /
/// `"outgoing"`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CallDirection {
    /// Callers of the queried symbol.
    Incoming,
    /// Callees the queried symbol invokes.
    Outgoing,
}

impl CallDirection {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CallHierarchyResponse {
    pub file_path: String,
    pub requested_position: PositionRecord,
    pub direction: CallDirection,
    pub found: bool,
    pub call_count: usize,
    pub calls: Vec<CallHierarchyCallRecord>,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct MacroExpansionResponse {
    pub file_path: String,
    pub requested_position: PositionRecord,
    pub found: bool,
    pub name: String,
    pub expansion: String,
    pub summary: String,
}

/// rust-analyzer's non-standard `rust-analyzer/expandMacro` request. Not part of
/// the LSP spec, so it isn't in `lsp_types::request`; we declare it here.
enum ExpandMacro {}

impl lsp_types::request::Request for ExpandMacro {
    type Params = lsp_types::TextDocumentPositionParams;
    type Result = Option<ExpandedMacro>;
    const METHOD: &'static str = "rust-analyzer/expandMacro";
}

/// rust-analyzer's `expandMacro` result payload. Internal LSP wire type (the
/// public MCP contract is `MacroExpansionResponse`, which inlines these fields),
/// so this is module-private and carries no `JsonSchema` derive. `Serialize` is
/// required by the `lsp_types::request::Request::Result` bound.
#[derive(Debug, Deserialize, Serialize)]
struct ExpandedMacro {
    name: String,
    expansion: String,
}

fn location_record(uri: &lsp_types::Uri, range: &lsp_types::Range) -> LocationRecord {
    let file_path = uri_to_path(uri);
    LocationRecord {
        display: format!(
            "{}:{}:{}",
            file_path,
            range.start.line.saturating_add(1),
            range.start.character.saturating_add(1),
        ),
        file_path,
        uri: uri.to_string(),
        line: range.start.line.saturating_add(1),
        column: range.start.character.saturating_add(1),
        end_line: range.end.line.saturating_add(1),
        end_column: range.end.character.saturating_add(1),
    }
}

// rust-analyzer can emit u32::MAX for synthetic / whole-line end positions;
// saturating_add keeps the +1 (zero-based to one-based) from panicking in debug
// or wrapping to 0 in release. These coordinates drive agent-applied edits, so a
// wrapped value would point an edit at the wrong span.
const fn range_record(range: &lsp_types::Range) -> RangeRecord {
    RangeRecord {
        start: PositionRecord {
            line: range.start.line.saturating_add(1),
            character: range.start.character.saturating_add(1),
        },
        end: PositionRecord {
            line: range.end.line.saturating_add(1),
            character: range.end.character.saturating_add(1),
        },
    }
}

fn text_edit_record(edit: &lsp_types::TextEdit) -> TextEditRecord {
    TextEditRecord {
        range: range_record(&edit.range),
        new_text: edit.new_text.clone(),
    }
}

/// Shape an LSP `WorkspaceEdit` into data. Handles both wire forms: the
/// `changes` map and `documentChanges` (rust-analyzer uses the latter for
/// renames). Files are sorted by path so the output is deterministic. Resource
/// operations (create/rename/delete file) carry no text edits and are skipped.
///
/// Per LSP, when a client advertises `documentChanges` support (we do), a
/// conforming server populates `document_changes` and omits `changes`. We honor
/// that precedence: if `document_changes` is present we use it exclusively, so a
/// non-conforming server that sets both does not double-count edits.
fn workspace_edit_record(edit: &lsp_types::WorkspaceEdit) -> WorkspaceEditRecord {
    let mut files: Vec<FileEditRecord> = Vec::new();

    if let Some(doc_changes) = &edit.document_changes {
        let doc_edits: Vec<&lsp_types::TextDocumentEdit> = match doc_changes {
            lsp_types::DocumentChanges::Edits(edits) => edits.iter().collect(),
            lsp_types::DocumentChanges::Operations(ops) => ops
                .iter()
                .filter_map(|op| match op {
                    lsp_types::DocumentChangeOperation::Edit(edit) => Some(edit),
                    lsp_types::DocumentChangeOperation::Op(_) => None,
                })
                .collect(),
        };
        for tde in doc_edits {
            let uri = &tde.text_document.uri;
            let edits = tde
                .edits
                .iter()
                .map(|one_of| match one_of {
                    lsp_types::OneOf::Left(text_edit) => text_edit_record(text_edit),
                    lsp_types::OneOf::Right(annotated) => text_edit_record(&annotated.text_edit),
                })
                .collect();
            files.push(FileEditRecord {
                file_path: uri_to_path(uri),
                uri: uri.to_string(),
                edits,
            });
        }
    } else if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            files.push(FileEditRecord {
                file_path: uri_to_path(uri),
                uri: uri.to_string(),
                edits: edits.iter().map(text_edit_record).collect(),
            });
        }
    }

    files.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    let edit_count = files.iter().map(|file| file.edits.len()).sum();
    WorkspaceEditRecord {
        file_count: files.len(),
        edit_count,
        files,
    }
}

fn code_action_record(item: lsp_types::CodeActionOrCommand) -> CodeActionRecord {
    match item {
        lsp_types::CodeActionOrCommand::Command(command) => CodeActionRecord {
            title: command.title,
            kind: None,
            is_preferred: false,
            command_arguments: command.arguments,
            command: Some(command.command),
            edit: None,
        },
        lsp_types::CodeActionOrCommand::CodeAction(action) => {
            let (command, command_arguments) = action
                .command
                .map_or((None, None), |cmd| (Some(cmd.command), cmd.arguments));
            CodeActionRecord {
                title: action.title,
                kind: action.kind.map(|kind| kind.as_str().to_string()),
                is_preferred: action.is_preferred.unwrap_or(false),
                command,
                command_arguments,
                edit: action.edit.as_ref().map(workspace_edit_record),
            }
        }
    }
}

/// Deepest symbol nesting we descend into before dropping `children`. Real Rust
/// files nest a handful of levels; this caps a pathological or hostile response
/// so a degenerate tree can't overflow the stack and crash the server.
const MAX_SYMBOL_DEPTH: usize = 64;

fn document_symbol_record(symbol: lsp_types::DocumentSymbol) -> DocumentSymbolRecord {
    fn to_record(symbol: lsp_types::DocumentSymbol, depth: usize) -> DocumentSymbolRecord {
        let children = if depth >= MAX_SYMBOL_DEPTH {
            vec![]
        } else {
            symbol
                .children
                .unwrap_or_default()
                .into_iter()
                .map(|child| to_record(child, depth + 1))
                .collect()
        };
        DocumentSymbolRecord {
            name: symbol.name,
            detail: symbol.detail,
            kind: symbol_kind_name(symbol.kind).to_string(),
            range: range_record(&symbol.range),
            selection_range: range_record(&symbol.selection_range),
            children,
        }
    }
    to_record(symbol, 0)
}

/// Shape a flat `SymbolInformation` into a (childless) symbol record. Flat
/// responses carry only a location, so the enclosing and selection ranges are
/// the same and there is no child nesting.
fn symbol_information_record(symbol: lsp_types::SymbolInformation) -> DocumentSymbolRecord {
    DocumentSymbolRecord {
        name: symbol.name,
        detail: None,
        kind: symbol_kind_name(symbol.kind).to_string(),
        range: range_record(&symbol.location.range),
        selection_range: range_record(&symbol.location.range),
        children: vec![],
    }
}

fn call_hierarchy_item_record(item: lsp_types::CallHierarchyItem) -> CallHierarchyItemRecord {
    CallHierarchyItemRecord {
        kind: symbol_kind_name(item.kind).to_string(),
        location: location_record(&item.uri, &item.selection_range),
        name: item.name,
        detail: item.detail,
    }
}

/// MCP server providing rust-analyzer tools via lspmux.
#[derive(Clone)]
pub struct RustAnalyzerTools {
    lsp: Arc<LspClient>,
    runtime_status: RuntimeStatus,
    telemetry: TelemetryState,
    config: lspmux_cc_mcp::bootstrap::RuntimeConfig,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl RustAnalyzerTools {
    /// Create a new tools instance wrapping an LSP client.
    pub fn new(
        lsp: Arc<LspClient>,
        runtime_status: RuntimeStatus,
        telemetry: TelemetryState,
        config: lspmux_cc_mcp::bootstrap::RuntimeConfig,
    ) -> Self {
        Self {
            lsp,
            runtime_status,
            telemetry,
            config,
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

        let locations = locations_from_goto_response(response);

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
        description = "Check rust-analyzer liveness, readiness, active workspace root, and shared lspmux bootstrap metadata."
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
        self.telemetry
            .refresh_compiler_accounting(workspace_root.as_deref());
        let readiness = self.lsp.readiness().await;
        let telemetry = self.telemetry.snapshot();
        let client = self.telemetry.client_identity();
        let compiler_accounting = self.telemetry.compiler_accounting_snapshot();
        // Refresh workspace-status fields by querying the daemon now. The
        // runtime_status snapshot was taken at MCP startup, before the
        // LspClient sent `initialize` and the daemon spawned an instance
        // for this workspace — in the cold-start path that snapshot's
        // workspace_match is stale by the time the tool is called.
        // client_engaged: whether this server has opened a file yet. Until it
        // has, the daemon hasn't been asked to spawn our instance, so a missing
        // match is "pending" (None), not a mismatch. See resolve_workspace_match.
        let client_engaged = self.lsp.has_opened_files().await;
        let fields = self
            .config
            .refresh_workspace_fields(self.runtime_status.service_mode, client_engaged)
            .await;
        let runtime = lspmux_cc_mcp::bootstrap::RuntimeStatus {
            // daemon_reachable must come from the live probe too: workspace_match
            // and workspace_match_label both read it, and a daemon that went down
            // after bootstrap would otherwise still read Some(true) from the
            // snapshot and mislabel an unreachable daemon as "pending".
            daemon_reachable: fields.daemon_reachable,
            served_workspaces: fields.served_workspaces,
            requested_workspace: fields.requested_workspace,
            workspace_match: fields.workspace_match,
            daemon_pid: fields.daemon_pid,
            daemon_idle_for_ms: fields.daemon_idle_for_ms,
            ..self.runtime_status.clone()
        };
        let workspace_match_str = workspace_match_label(
            runtime.workspace_match,
            runtime.daemon_reachable,
            runtime.requested_workspace.is_some(),
        );
        let summary = format!(
            "{SERVER_NAME} liveness: {server_status}; readiness: {}; workspace root: {}; \
             workspace match: {workspace_match_str}",
            readiness.health,
            workspace_root
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string())
        );

        Ok(Json(ServerStatusResponse {
            server: SERVER_NAME.to_string(),
            server_status: server_status.to_string(),
            workspace_root,
            server_version,
            runtime,
            client,
            readiness,
            telemetry,
            compiler_accounting,
            summary,
        }))
    }

    /// Return the full lspmux instance registry — all workspaces with active
    /// rust-analyzer instances under the daemon, with their pids, idle times,
    /// and client counts.
    #[tool(
        name = "rust_workspace_registry",
        description = "List all rust-analyzer instances the lspmux daemon is currently hosting. Each entry includes pid, workspace_root (raw + canonical), idle_for_ms, and client_count. Use this to debug which workspaces are active, find stale instances, or confirm your workspace has a dedicated rust-analyzer."
    )]
    async fn workspace_registry(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<RegistrySnapshot>, McpError> {
        // Re-query rather than caching: the registry is dynamic (instances
        // come and go via idle timeouts) and the user calls this tool
        // precisely when they want current state.
        let instances_opt = self.config.discover_status().await;
        let daemon_reachable = instances_opt.is_some();
        let instances = instances_opt.unwrap_or_default();
        let instance_count = instances.len();
        let summary = if daemon_reachable {
            if instance_count > 0 {
                format!("lspmux daemon hosting {instance_count} rust-analyzer instance(s).")
            } else {
                "lspmux daemon reachable, hosting no instances.".to_string()
            }
        } else {
            "lspmux daemon unreachable.".to_string()
        };
        Ok(Json(RegistrySnapshot {
            daemon_reachable,
            instances,
            instance_count,
            summary,
        }))
    }

    /// List code actions (quick fixes, refactors) for a range.
    #[tool(
        name = "rust_code_actions",
        description = "List the code actions (quick fixes, refactors) rust-analyzer offers for a range in a file. Each action carries its title, kind, and workspace edit as data; edits are NOT applied, so apply them yourself. Diagnostic-triggered quick fixes may be absent: this sends an empty diagnostics context, so actions keyed to a specific diagnostic at the range are not requested. Input line/character are zero-based; returned ranges are one-based."
    )]
    async fn code_actions(
        &self,
        params: Parameters<RangeParam>,
    ) -> Result<Json<CodeActionsResponse>, McpError> {
        let p = &params.0;
        let uri = self.open_file_uri(&p.file_path).await?;

        let range = lsp_types::Range {
            start: lsp_types::Position::new(p.start_line, p.start_character),
            end: lsp_types::Position::new(p.end_line, p.end_character),
        };
        let action_params = lsp_types::CodeActionParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            range,
            context: lsp_types::CodeActionContext {
                diagnostics: vec![],
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        };

        let response = self
            .lsp
            .request::<lsp_types::request::CodeActionRequest>(action_params)
            .await
            .map_err(|e| internal_error(format!("code action request failed: {e}")))?;

        let actions = response
            .unwrap_or_default()
            .into_iter()
            .map(code_action_record)
            .collect::<Vec<_>>();
        let action_count = actions.len();
        let summary = if action_count == 0 {
            format!("No code actions available for {}.", p.file_path)
        } else {
            format!("Found {action_count} code action(s) for {}.", p.file_path)
        };

        Ok(Json(CodeActionsResponse {
            file_path: p.file_path.clone(),
            // Echo the request range as the raw zero-based input, matching the
            // requested_position convention used by the other tools (the
            // one-based form is reserved for returned/result locations).
            requested_range: RangeRecord {
                start: PositionRecord {
                    line: p.start_line,
                    character: p.start_character,
                },
                end: PositionRecord {
                    line: p.end_line,
                    character: p.end_character,
                },
            },
            action_count,
            actions,
            summary,
        }))
    }

    /// Return a file's hierarchical symbol tree.
    #[tool(
        name = "rust_document_symbols",
        description = "Outline a Rust file's symbols (modules, functions, structs, impls) as a tree without reading the file. Returns nested records with one-based ranges."
    )]
    async fn document_symbols(
        &self,
        params: Parameters<FileParam>,
    ) -> Result<Json<DocumentSymbolsResponse>, McpError> {
        let file = &params.0.file_path;
        let uri = self.open_file_uri(file).await?;

        let symbol_params = lsp_types::DocumentSymbolParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        };
        let response = self
            .lsp
            .request::<lsp_types::request::DocumentSymbolRequest>(symbol_params)
            .await
            .map_err(|e| internal_error(format!("document symbol request failed: {e}")))?;

        let symbols = match response {
            Some(lsp_types::DocumentSymbolResponse::Nested(symbols)) => {
                symbols.into_iter().map(document_symbol_record).collect()
            }
            Some(lsp_types::DocumentSymbolResponse::Flat(symbols)) => {
                symbols.into_iter().map(symbol_information_record).collect()
            }
            None => vec![],
        };

        let symbol_count = symbols.len();
        let summary = if symbol_count == 0 {
            format!("No symbols found in {file}.")
        } else {
            format!("Found {symbol_count} top-level symbol(s) in {file}.")
        };

        Ok(Json(DocumentSymbolsResponse {
            file_path: file.clone(),
            symbol_count,
            symbols,
            summary,
        }))
    }

    /// Compute the workspace edit for a rename without applying it.
    #[tool(
        name = "rust_rename",
        description = "Compute the workspace edit to rename a symbol across the codebase. Returns the per-file text edits as data; NOTHING is written to disk, so apply the edits yourself. Input line/character are zero-based; returned ranges are one-based."
    )]
    async fn rename(
        &self,
        params: Parameters<RenameParam>,
    ) -> Result<Json<RenameResponse>, McpError> {
        let p = &params.0;
        if p.new_name.trim().is_empty() {
            return Err(McpError::invalid_params(
                "new_name must not be empty".to_string(),
                None,
            ));
        }
        let uri = self.open_file_uri(&p.file_path).await?;

        let rename_params = lsp_types::RenameParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(p.line, p.character),
            },
            new_name: p.new_name.clone(),
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
        };
        let response = self
            .lsp
            .request::<lsp_types::request::Rename>(rename_params)
            .await
            .map_err(|e| internal_error(format!("rename request failed: {e}")))?;

        let edit = response.as_ref().map(workspace_edit_record);
        let found = edit.as_ref().is_some_and(|e| e.file_count > 0);
        let summary = match &edit {
            Some(edit) if found => format!(
                "Rename to {:?} touches {} file(s), {} edit(s). Not applied; apply the edits yourself.",
                p.new_name, edit.file_count, edit.edit_count
            ),
            _ => format!("No rename edits produced at this position for {:?}.", p.new_name),
        };

        Ok(Json(RenameResponse {
            file_path: p.file_path.clone(),
            requested_position: PositionRecord {
                line: p.line,
                character: p.character,
            },
            new_name: p.new_name.clone(),
            found,
            edit,
            summary,
        }))
    }

    /// Find the implementations of a trait or trait method.
    #[tool(
        name = "rust_goto_implementation",
        description = "Find implementations of a trait, trait method, or symbol at a position (textDocument/implementation). Returns one-based file locations, distinct from go-to-definition."
    )]
    async fn goto_implementation(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<Json<LocationsResponse>, McpError> {
        let p = &params.0;
        let uri = self.open_file_uri(&p.file_path).await?;

        let impl_params = lsp_types::request::GotoImplementationParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(p.line, p.character),
            },
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        };
        let response = self
            .lsp
            .request::<lsp_types::request::GotoImplementation>(impl_params)
            .await
            .map_err(|e| internal_error(format!("go to implementation failed: {e}")))?;

        let locations = locations_from_goto_response(response);

        let found = !locations.is_empty();
        let location_count = locations.len();
        let summary = if found {
            format!("Found {location_count} implementation location(s).")
        } else {
            "No implementations found at this position.".to_string()
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

    /// Find the callers of the symbol at a position.
    #[tool(
        name = "rust_call_hierarchy_incoming",
        description = "Find the callers of the function/method at a position (prepareCallHierarchy + incomingCalls). Returns each caller and the one-based call-site ranges within it."
    )]
    async fn call_hierarchy_incoming(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<Json<CallHierarchyResponse>, McpError> {
        let p = &params.0;
        let item = self
            .prepare_call_hierarchy(&p.file_path, p.line, p.character)
            .await?;
        let calls = match item {
            None => vec![],
            Some(item) => {
                let call_params = lsp_types::CallHierarchyIncomingCallsParams {
                    item,
                    work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                    partial_result_params: lsp_types::PartialResultParams::default(),
                };
                self.lsp
                    .request::<lsp_types::request::CallHierarchyIncomingCalls>(call_params)
                    .await
                    .map_err(|e| internal_error(format!("incoming calls request failed: {e}")))?
                    .unwrap_or_default()
                    .into_iter()
                    .map(|call| CallHierarchyCallRecord {
                        from_ranges: call.from_ranges.iter().map(range_record).collect(),
                        item: call_hierarchy_item_record(call.from),
                    })
                    .collect()
            }
        };
        Ok(Json(call_hierarchy_response(
            p,
            CallDirection::Incoming,
            calls,
        )))
    }

    /// Find the calls made by the symbol at a position.
    #[tool(
        name = "rust_call_hierarchy_outgoing",
        description = "Find the functions/methods called by the symbol at a position (prepareCallHierarchy + outgoingCalls). Returns each callee and the one-based call-site ranges within the queried symbol."
    )]
    async fn call_hierarchy_outgoing(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<Json<CallHierarchyResponse>, McpError> {
        let p = &params.0;
        let item = self
            .prepare_call_hierarchy(&p.file_path, p.line, p.character)
            .await?;
        let calls = match item {
            None => vec![],
            Some(item) => {
                let call_params = lsp_types::CallHierarchyOutgoingCallsParams {
                    item,
                    work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                    partial_result_params: lsp_types::PartialResultParams::default(),
                };
                self.lsp
                    .request::<lsp_types::request::CallHierarchyOutgoingCalls>(call_params)
                    .await
                    .map_err(|e| internal_error(format!("outgoing calls request failed: {e}")))?
                    .unwrap_or_default()
                    .into_iter()
                    .map(|call| CallHierarchyCallRecord {
                        from_ranges: call.from_ranges.iter().map(range_record).collect(),
                        item: call_hierarchy_item_record(call.to),
                    })
                    .collect()
            }
        };
        Ok(Json(call_hierarchy_response(
            p,
            CallDirection::Outgoing,
            calls,
        )))
    }

    /// Expand the macro at a position.
    #[tool(
        name = "rust_expand_macro",
        description = "Expand the macro invocation at a position (rust-analyzer/expandMacro). Returns the macro name and its expanded source text. found=false when the position is not on a macro."
    )]
    async fn expand_macro(
        &self,
        params: Parameters<PositionParam>,
    ) -> Result<Json<MacroExpansionResponse>, McpError> {
        let p = &params.0;
        let uri = self.open_file_uri(&p.file_path).await?;

        let expand_params = lsp_types::TextDocumentPositionParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            position: lsp_types::Position::new(p.line, p.character),
        };
        let response = self
            .lsp
            .request::<ExpandMacro>(expand_params)
            .await
            .map_err(|e| internal_error(format!("expand macro request failed: {e}")))?;

        let requested_position = PositionRecord {
            line: p.line,
            character: p.character,
        };
        let macro_response = match response {
            Some(expanded) => MacroExpansionResponse {
                file_path: p.file_path.clone(),
                requested_position,
                found: true,
                summary: format!("Expanded macro {:?}.", expanded.name),
                name: expanded.name,
                expansion: expanded.expansion,
            },
            None => MacroExpansionResponse {
                file_path: p.file_path.clone(),
                requested_position,
                found: false,
                name: String::new(),
                expansion: String::new(),
                summary: "No macro to expand at this position.".to_string(),
            },
        };
        Ok(Json(macro_response))
    }

    /// Validate the path, sync the file into the daemon, and build its URI.
    /// Shared prelude for the tools that issue a position/range LSP request.
    async fn open_file_uri(&self, file: &str) -> Result<lsp_types::Uri, McpError> {
        validate_file_path(file)?;
        self.lsp
            .ensure_file_open(file)
            .await
            .map_err(|e| internal_error(format!("failed to synchronize file with lspmux: {e}")))?;
        file_uri(file)
            .map_err(|e| McpError::invalid_params(format!("invalid file path: {e}"), None))
    }

    /// Prepare a call hierarchy at a position. Opens the file and runs
    /// `textDocument/prepareCallHierarchy`, returning the anchor item if any.
    async fn prepare_call_hierarchy(
        &self,
        file: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::CallHierarchyItem>, McpError> {
        let uri = self.open_file_uri(file).await?;

        let prepare_params = lsp_types::CallHierarchyPrepareParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(line, character),
            },
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
        };
        let items = self
            .lsp
            .request::<lsp_types::request::CallHierarchyPrepare>(prepare_params)
            .await
            .map_err(|e| internal_error(format!("prepare call hierarchy failed: {e}")))?;

        Ok(items.and_then(|mut items| (!items.is_empty()).then(|| items.remove(0))))
    }
}

/// Shape a `textDocument/definition`-style response (definition or
/// implementation, both `GotoDefinitionResponse`) into one-based location records.
fn locations_from_goto_response(
    response: Option<lsp_types::GotoDefinitionResponse>,
) -> Vec<LocationRecord> {
    match response {
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
    }
}

/// Build a `CallHierarchyResponse` from shaped call records.
fn call_hierarchy_response(
    p: &PositionParam,
    direction: CallDirection,
    calls: Vec<CallHierarchyCallRecord>,
) -> CallHierarchyResponse {
    let found = !calls.is_empty();
    let call_count = calls.len();
    let label = direction.as_str();
    let summary = if found {
        format!("Found {call_count} {label} call(s).")
    } else {
        format!("No {label} calls found at this position.")
    };
    CallHierarchyResponse {
        file_path: p.file_path.clone(),
        requested_position: PositionRecord {
            line: p.line,
            character: p.character,
        },
        direction,
        found,
        call_count,
        calls,
        summary,
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
        let tool_name = request.name.clone();
        let client = self.telemetry.client_identity();
        let started = Instant::now();
        tracing::info!(
            event = "tool_start",
            tool = %tool_name,
            client_kind = %client.kind,
            client_host = %client.host,
            session_id = %client.session_id
        );
        let ctx = ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(ctx).await;
        let latency_ms = started.elapsed().as_millis();
        let latency_ms_u64 = u64::try_from(latency_ms).unwrap_or(u64::MAX);

        match &result {
            Ok(_) => {
                self.telemetry.record_tool_result(
                    &tool_name,
                    ToolOutcome::Success,
                    latency_ms_u64,
                    None,
                    None,
                );
                tracing::info!(
                    event = "tool_result",
                    tool = %tool_name,
                    outcome = "success",
                    latency_ms = latency_ms
                );
                if tool_name != "rust_server_status" {
                    let workspace_root = self.lsp.workspace_root().await;
                    self.telemetry
                        .refresh_compiler_accounting(workspace_root.as_deref());
                }
            }
            Err(error) => {
                let outcome = classify_tool_error(error);
                self.telemetry.record_tool_result(
                    &tool_name,
                    outcome,
                    latency_ms_u64,
                    Some(error_code_name(error.code)),
                    Some(&error.message),
                );
                tracing::warn!(
                    event = "tool_result",
                    tool = %tool_name,
                    outcome = %outcome.as_str(),
                    error_code = ?error.code,
                    error = %error.message,
                    latency_ms = latency_ms
                );
            }
        }

        result
    }
}

/// Human-readable `workspace_match` label for the status summary.
///
/// Both "instance pending" and "daemon unreachable" surface as
/// `workspace_match: None`; this splits them for the reader using
/// `daemon_reachable`. A reachable daemon with a requested workspace and no
/// determination yet is "pending" (the cold-start window before this client
/// opens a file); everything else undeterminable is "unknown".
const fn workspace_match_label(
    workspace_match: Option<bool>,
    daemon_reachable: Option<bool>,
    workspace_requested: bool,
) -> &'static str {
    match workspace_match {
        Some(true) => "true",
        Some(false) => "false",
        None if matches!(daemon_reachable, Some(true)) && workspace_requested => "pending",
        None => "unknown",
    }
}

fn classify_tool_error(error: &McpError) -> ToolOutcome {
    if error.code == ErrorCode::INVALID_PARAMS {
        ToolOutcome::InvalidParams
    } else if error.message.contains("timed out") {
        ToolOutcome::Timeout
    } else {
        ToolOutcome::Failure
    }
}

const fn error_code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::PARSE_ERROR => "parse_error",
        ErrorCode::INVALID_REQUEST => "invalid_request",
        ErrorCode::METHOD_NOT_FOUND => "method_not_found",
        ErrorCode::INVALID_PARAMS => "invalid_params",
        ErrorCode::INTERNAL_ERROR => "internal_error",
        ErrorCode::RESOURCE_NOT_FOUND => "resource_not_found",
        ErrorCode::URL_ELICITATION_REQUIRED => "url_elicitation_required",
        _ => "other",
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
    fn workspace_match_label_splits_pending_from_unknown() {
        assert_eq!(workspace_match_label(Some(true), Some(true), true), "true");
        assert_eq!(
            workspace_match_label(Some(false), Some(true), true),
            "false"
        );
        // Reachable + requested + undetermined => cold-start pending.
        assert_eq!(workspace_match_label(None, Some(true), true), "pending");
        // Daemon down => unknown, not pending.
        assert_eq!(workspace_match_label(None, Some(false), true), "unknown");
        // Probe skipped => unknown.
        assert_eq!(workspace_match_label(None, None, true), "unknown");
        // Reachable but no workspace requested => unknown, not pending.
        assert_eq!(workspace_match_label(None, Some(true), false), "unknown");
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

    fn test_uri(path: &str) -> lsp_types::Uri {
        lspmux_cc_mcp::lsp_client::file_uri(path).unwrap()
    }

    fn test_range(sl: u32, sc: u32, el: u32, ec: u32) -> lsp_types::Range {
        lsp_types::Range {
            start: lsp_types::Position::new(sl, sc),
            end: lsp_types::Position::new(el, ec),
        }
    }

    #[test]
    fn range_param_deserializes() {
        let json = serde_json::json!({
            "file_path": "/abs/x.rs",
            "start_line": 1, "start_character": 2,
            "end_line": 3, "end_character": 4
        });
        let p: RangeParam = serde_json::from_value(json).unwrap();
        assert_eq!(p.start_line, 1);
        assert_eq!(p.end_character, 4);
    }

    #[test]
    fn rename_param_deserializes() {
        let json = serde_json::json!({
            "file_path": "/abs/x.rs", "line": 5, "character": 6, "new_name": "renamed"
        });
        let p: RenameParam = serde_json::from_value(json).unwrap();
        assert_eq!(p.new_name, "renamed");
    }

    // lsp_types::Uri carries interior mutability, which clippy flags as a map
    // key; it is the key type of WorkspaceEdit::changes upstream, so the test
    // mirrors the real shape.
    #[allow(clippy::mutable_key_type)]
    #[test]
    fn workspace_edit_record_from_changes_is_sorted_and_one_based() {
        let mut changes = std::collections::HashMap::new();
        changes.insert(
            test_uri("/b.rs"),
            vec![lsp_types::TextEdit {
                range: test_range(0, 0, 0, 3),
                new_text: "foo".to_string(),
            }],
        );
        changes.insert(
            test_uri("/a.rs"),
            vec![
                lsp_types::TextEdit {
                    range: test_range(1, 1, 1, 2),
                    new_text: "x".to_string(),
                },
                lsp_types::TextEdit {
                    range: test_range(2, 0, 2, 1),
                    new_text: "y".to_string(),
                },
            ],
        );
        let edit = lsp_types::WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        };
        let record = workspace_edit_record(&edit);
        assert_eq!(record.file_count, 2);
        assert_eq!(record.edit_count, 3);
        // Files are sorted by path for deterministic output.
        assert_eq!(record.files[0].file_path, "/a.rs");
        assert_eq!(record.files[1].file_path, "/b.rs");
        // Ranges are one-based.
        assert_eq!(record.files[1].edits[0].range.start.line, 1);
        assert_eq!(record.files[1].edits[0].range.start.character, 1);
        assert_eq!(record.files[1].edits[0].new_text, "foo");
    }

    #[test]
    fn workspace_edit_record_from_document_changes() {
        let tde = lsp_types::TextDocumentEdit {
            text_document: lsp_types::OptionalVersionedTextDocumentIdentifier {
                uri: test_uri("/c.rs"),
                version: Some(1),
            },
            edits: vec![lsp_types::OneOf::Left(lsp_types::TextEdit {
                range: test_range(0, 0, 0, 1),
                new_text: "z".to_string(),
            })],
        };
        let edit = lsp_types::WorkspaceEdit {
            changes: None,
            document_changes: Some(lsp_types::DocumentChanges::Edits(vec![tde])),
            change_annotations: None,
        };
        let record = workspace_edit_record(&edit);
        assert_eq!(record.file_count, 1);
        assert_eq!(record.edit_count, 1);
        assert_eq!(record.files[0].file_path, "/c.rs");
        assert_eq!(record.files[0].edits[0].new_text, "z");
    }

    #[allow(clippy::mutable_key_type)] // see workspace_edit_record_from_changes note
    #[test]
    fn code_action_record_distinguishes_command_from_edit() {
        // A command-only action: command id present, no edit, no kind.
        let command = lsp_types::CodeActionOrCommand::Command(lsp_types::Command {
            title: "Run it".to_string(),
            command: "rust-analyzer.run".to_string(),
            arguments: None,
        });
        let rec = code_action_record(command);
        assert_eq!(rec.command.as_deref(), Some("rust-analyzer.run"));
        assert!(rec.edit.is_none());
        assert!(rec.kind.is_none());

        // An edit-bearing quick fix: kind + edit present, no command.
        let mut changes = std::collections::HashMap::new();
        changes.insert(
            test_uri("/a.rs"),
            vec![lsp_types::TextEdit {
                range: test_range(0, 0, 0, 1),
                new_text: "q".to_string(),
            }],
        );
        let action = lsp_types::CodeActionOrCommand::CodeAction(lsp_types::CodeAction {
            title: "Quick fix".to_string(),
            kind: Some(lsp_types::CodeActionKind::QUICKFIX),
            is_preferred: Some(true),
            edit: Some(lsp_types::WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            }),
            ..Default::default()
        });
        let rec = code_action_record(action);
        assert_eq!(rec.kind.as_deref(), Some("quickfix"));
        assert!(rec.is_preferred);
        assert!(rec.command.is_none());
        assert_eq!(rec.edit.unwrap().edit_count, 1);
    }

    #[test]
    fn document_symbol_record_nests_children() {
        // The `deprecated` field is deprecated upstream in lsp_types but is a
        // required field for constructing the struct literal.
        #[allow(deprecated)]
        let child = lsp_types::DocumentSymbol {
            name: "inner_fn".to_string(),
            detail: Some("fn()".to_string()),
            kind: lsp_types::SymbolKind::FUNCTION,
            tags: None,
            deprecated: None,
            range: test_range(1, 0, 5, 1),
            selection_range: test_range(1, 3, 1, 11),
            children: None,
        };
        // The `deprecated` field is deprecated upstream in lsp_types but is a
        // required field for constructing the struct literal.
        #[allow(deprecated)]
        let parent = lsp_types::DocumentSymbol {
            name: "my_mod".to_string(),
            detail: None,
            kind: lsp_types::SymbolKind::MODULE,
            tags: None,
            deprecated: None,
            range: test_range(0, 0, 6, 1),
            selection_range: test_range(0, 4, 0, 10),
            children: Some(vec![child]),
        };
        let rec = document_symbol_record(parent);
        assert_eq!(rec.name, "my_mod");
        assert_eq!(rec.kind, "module");
        assert_eq!(rec.children.len(), 1);
        assert_eq!(rec.children[0].name, "inner_fn");
        assert_eq!(rec.children[0].kind, "function");
        // One-based: input line 1 -> 2.
        assert_eq!(rec.children[0].range.start.line, 2);
    }

    #[test]
    fn document_symbol_record_caps_deep_nesting() {
        // Build a single chain far deeper than MAX_SYMBOL_DEPTH. A naive
        // recursive shaper would descend the whole chain (and a hostile tree
        // could overflow the stack); the cap drops children at the limit.
        #[allow(deprecated)]
        fn deep_symbol(remaining: usize) -> lsp_types::DocumentSymbol {
            lsp_types::DocumentSymbol {
                name: format!("level_{remaining}"),
                detail: None,
                kind: lsp_types::SymbolKind::MODULE,
                tags: None,
                deprecated: None,
                range: test_range(0, 0, 1, 0),
                selection_range: test_range(0, 0, 0, 1),
                children: if remaining == 0 {
                    None
                } else {
                    Some(vec![deep_symbol(remaining - 1)])
                },
            }
        }

        let depth = MAX_SYMBOL_DEPTH + 50;
        let rec = document_symbol_record(deep_symbol(depth));

        // Walk the shaped record and count how deep nesting actually goes.
        let mut node = &rec;
        let mut levels = 1;
        while let Some(child) = node.children.first() {
            node = child;
            levels += 1;
        }
        // The node at depth == MAX_SYMBOL_DEPTH is shaped but has its children
        // dropped, so the retained chain spans depths 0..=MAX_SYMBOL_DEPTH.
        assert_eq!(levels, MAX_SYMBOL_DEPTH + 1);
        assert!(node.children.is_empty());
    }

    #[test]
    fn symbol_information_record_is_flat_and_childless() {
        // The `deprecated` field is deprecated upstream in lsp_types but is a
        // required field for constructing the struct literal.
        #[allow(deprecated)]
        let info = lsp_types::SymbolInformation {
            name: "TopLevel".to_string(),
            kind: lsp_types::SymbolKind::STRUCT,
            tags: None,
            deprecated: None,
            location: lsp_types::Location {
                uri: test_uri("/a.rs"),
                range: test_range(3, 0, 3, 8),
            },
            container_name: None,
        };
        let rec = symbol_information_record(info);
        assert_eq!(rec.kind, "struct");
        assert!(rec.children.is_empty());
        assert_eq!(rec.range.start.line, 4);
    }

    #[test]
    fn call_hierarchy_item_record_uses_selection_range() {
        let item = lsp_types::CallHierarchyItem {
            name: "callee".to_string(),
            kind: lsp_types::SymbolKind::FUNCTION,
            tags: None,
            detail: Some("fn callee()".to_string()),
            uri: test_uri("/a.rs"),
            range: test_range(0, 0, 10, 1),
            selection_range: test_range(0, 3, 0, 9),
            data: None,
        };
        let rec = call_hierarchy_item_record(item);
        assert_eq!(rec.name, "callee");
        assert_eq!(rec.kind, "function");
        // Location anchors on selection_range, one-based.
        assert_eq!(rec.location.line, 1);
        assert_eq!(rec.location.column, 4);
    }

    #[test]
    fn expand_macro_request_method_and_result() {
        use lsp_types::request::Request;
        assert_eq!(ExpandMacro::METHOD, "rust-analyzer/expandMacro");
        let value =
            serde_json::json!({ "name": "vec", "expansion": "{ let mut v = Vec::new(); v }" });
        let expanded: ExpandedMacro = serde_json::from_value(value).unwrap();
        assert_eq!(expanded.name, "vec");
        assert!(expanded.expansion.contains("Vec::new"));
    }

    #[test]
    fn call_hierarchy_response_reports_empty() {
        let p = PositionParam {
            file_path: "/a.rs".to_string(),
            line: 1,
            character: 2,
        };
        let response = call_hierarchy_response(&p, CallDirection::Incoming, vec![]);
        assert!(!response.found);
        assert_eq!(response.call_count, 0);
        assert_eq!(response.direction, CallDirection::Incoming);
        assert!(response.summary.contains("No incoming calls"));
    }

    #[test]
    fn call_hierarchy_response_found_branch() {
        let p = PositionParam {
            file_path: "/a.rs".to_string(),
            line: 1,
            character: 2,
        };
        let call = CallHierarchyCallRecord {
            item: CallHierarchyItemRecord {
                name: "caller".to_string(),
                kind: "function".to_string(),
                detail: None,
                location: location_record(&test_uri("/a.rs"), &test_range(0, 0, 0, 1)),
            },
            from_ranges: vec![range_record(&test_range(2, 0, 2, 4))],
        };
        let response = call_hierarchy_response(&p, CallDirection::Outgoing, vec![call]);
        assert!(response.found);
        assert_eq!(response.call_count, 1);
        assert_eq!(response.direction, CallDirection::Outgoing);
        assert!(response.summary.contains("Found 1 outgoing call"));
    }

    // When a server (non-conformingly) sets both changes and document_changes,
    // document_changes wins; the changes map is ignored, so edits are not
    // double-counted.
    #[allow(clippy::mutable_key_type)] // see workspace_edit_record_from_changes note
    #[test]
    fn workspace_edit_record_prefers_document_changes() {
        let mut changes = std::collections::HashMap::new();
        changes.insert(
            test_uri("/from_changes.rs"),
            vec![lsp_types::TextEdit {
                range: test_range(0, 0, 0, 1),
                new_text: "ignored".to_string(),
            }],
        );
        let tde = lsp_types::TextDocumentEdit {
            text_document: lsp_types::OptionalVersionedTextDocumentIdentifier {
                uri: test_uri("/from_doc_changes.rs"),
                version: Some(1),
            },
            edits: vec![lsp_types::OneOf::Left(lsp_types::TextEdit {
                range: test_range(0, 0, 0, 1),
                new_text: "kept".to_string(),
            })],
        };
        let edit = lsp_types::WorkspaceEdit {
            changes: Some(changes),
            document_changes: Some(lsp_types::DocumentChanges::Edits(vec![tde])),
            change_annotations: None,
        };
        let record = workspace_edit_record(&edit);
        assert_eq!(record.file_count, 1);
        assert_eq!(record.edit_count, 1);
        assert_eq!(record.files[0].file_path, "/from_doc_changes.rs");
        assert_eq!(record.files[0].edits[0].new_text, "kept");
    }

    #[test]
    fn workspace_edit_record_operations_skips_resource_ops() {
        let create = lsp_types::DocumentChangeOperation::Op(lsp_types::ResourceOp::Create(
            lsp_types::CreateFile {
                uri: test_uri("/new_file.rs"),
                options: None,
                annotation_id: None,
            },
        ));
        let edit_op = lsp_types::DocumentChangeOperation::Edit(lsp_types::TextDocumentEdit {
            text_document: lsp_types::OptionalVersionedTextDocumentIdentifier {
                uri: test_uri("/edited.rs"),
                version: Some(1),
            },
            edits: vec![lsp_types::OneOf::Left(lsp_types::TextEdit {
                range: test_range(0, 0, 0, 1),
                new_text: "x".to_string(),
            })],
        });
        let edit = lsp_types::WorkspaceEdit {
            changes: None,
            document_changes: Some(lsp_types::DocumentChanges::Operations(vec![
                create, edit_op,
            ])),
            change_annotations: None,
        };
        let record = workspace_edit_record(&edit);
        // The resource op is skipped; only the text-edit document is shaped.
        assert_eq!(record.file_count, 1);
        assert_eq!(record.edit_count, 1);
        assert_eq!(record.files[0].file_path, "/edited.rs");
    }

    #[test]
    fn workspace_edit_record_handles_annotated_text_edit() {
        let annotated = lsp_types::AnnotatedTextEdit {
            text_edit: lsp_types::TextEdit {
                range: test_range(1, 0, 1, 2),
                new_text: "annotated".to_string(),
            },
            annotation_id: "rename-1".to_string(),
        };
        let tde = lsp_types::TextDocumentEdit {
            text_document: lsp_types::OptionalVersionedTextDocumentIdentifier {
                uri: test_uri("/a.rs"),
                version: Some(1),
            },
            edits: vec![lsp_types::OneOf::Right(annotated)],
        };
        let edit = lsp_types::WorkspaceEdit {
            changes: None,
            document_changes: Some(lsp_types::DocumentChanges::Edits(vec![tde])),
            change_annotations: None,
        };
        let record = workspace_edit_record(&edit);
        assert_eq!(record.edit_count, 1);
        assert_eq!(record.files[0].edits[0].new_text, "annotated");
        // One-based: input line 1 -> 2.
        assert_eq!(record.files[0].edits[0].range.start.line, 2);
    }

    #[test]
    fn code_action_record_preserves_command_arguments_and_both_edit_and_command() {
        // A code action that carries BOTH an edit and a command with arguments,
        // and leaves is_preferred unset (defaults to false).
        let action = lsp_types::CodeActionOrCommand::CodeAction(lsp_types::CodeAction {
            title: "Run and edit".to_string(),
            kind: Some(lsp_types::CodeActionKind::REFACTOR),
            command: Some(lsp_types::Command {
                title: "Run".to_string(),
                command: "rust-analyzer.runSingle".to_string(),
                arguments: Some(vec![serde_json::json!({ "label": "test" })]),
            }),
            ..Default::default()
        });
        let rec = code_action_record(action);
        assert_eq!(rec.command.as_deref(), Some("rust-analyzer.runSingle"));
        assert_eq!(
            rec.command_arguments.as_ref().map(Vec::len),
            Some(1),
            "command arguments preserved"
        );
        assert!(
            !rec.is_preferred,
            "is_preferred defaults to false when None"
        );
        assert!(rec.edit.is_none());
    }
}
