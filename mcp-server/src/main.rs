//! lspmux-cc-mcp: MCP server providing rust-analyzer tools via lspmux.
//!
//! Architecture:
//! ```text
//! Any MCP host <-MCP (stdio)-> lspmux-cc-mcp <-LSP (child stdio)-> lspmux client <-socket-> lspmux server -> rust-analyzer
//! ```

mod tools;

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use lspmux_cc_mcp::bootstrap::{RuntimeConfig, SERVER_NAME};
use lspmux_cc_mcp::lsp_client::LspClient;
use lspmux_cc_mcp::telemetry::TelemetryState;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ServerCapabilities, ServerInfo, ToolsCapability,
};
use rmcp::service::{RequestContext, ServiceExt};
use rmcp::transport::io::stdio;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::tools::RustAnalyzerTools;

/// MCP server wrapping the rust-analyzer tools.
#[derive(Clone)]
struct LspmuxMcpServer {
    tools: RustAnalyzerTools,
}

impl ServerHandler for LspmuxMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: rmcp::model::Implementation {
                name: env!("CARGO_PKG_NAME").into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            instructions: Some(
                "Provides Rust development intelligence over MCP by talking to a shared \
                 rust-analyzer instance through lspmux.\n\
                 \n\
                 Tools:\n\
                 - rust_diagnostics(file_path): compiler errors and warnings for a file\n\
                 - rust_hover(file_path, line, character): type info and docs at a position\n\
                 - rust_goto_definition(file_path, line, character): find definition location\n\
                 - rust_find_references(file_path, line, character): find all references\n\
                 - rust_workspace_symbol(query): find symbols by name across the workspace\n\
                 - rust_server_status(): check server health and active workspace root\n\
                 \n\
                 Position format: line and character inputs are ZERO-BASED (first line = 0).\n\
                 Output locations (file:line:col) are ONE-BASED. Subtract 1 from each before\n\
                 using as input to another tool.\n\
                 \n\
                 Workflow: run rust_diagnostics after edits to check for errors. If results\n\
                 seem stale, use rust_server_status to check readiness instead of guessing.\n\
                 All file paths must be absolute. Tools are read-only and workspace-scoped.\n\
                 Use rust_server_status to confirm the correct workspace root and shared-service \
                 bootstrap state."
                    .into(),
            ),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: None }),
                ..ServerCapabilities::default()
            },
            ..ServerInfo::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<rmcp::model::ListToolsResult, McpError> {
        Ok(self.tools.list_tools())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, McpError> {
        self.tools.call_tool(request, context).await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing to stderr (stdout is MCP transport)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let runtime = RuntimeConfig::discover().context("failed to resolve runtime configuration")?;
    if std::env::var("WORKSPACE_ROOT").is_err() {
        tracing::warn!(
            "WORKSPACE_ROOT env var not set; using current_dir as fallback: {:?}. \
             Set WORKSPACE_ROOT in your MCP client env for deterministic workspace detection.",
            runtime.workspace_root
        );
    } else {
        tracing::info!("workspace root: {:?}", runtime.workspace_root);
    }

    tracing::info!("Starting lspmux-cc-mcp server");
    tracing::info!("lspmux binary: {}", runtime.lspmux_path);
    tracing::info!("{SERVER_NAME} binary: {}", runtime.server_path);

    let telemetry = TelemetryState::from_env();
    tracing::info!(
        event = "client_identity",
        client_kind = %telemetry.client_identity().kind,
        client_host = %telemetry.client_identity().host,
        session_id = %telemetry.client_identity().session_id
    );

    let bootstrap_started = Instant::now();
    let runtime_status = match runtime.ensure_service_running().await {
        Ok(status) => {
            let bootstrap_latency_ms =
                u64::try_from(bootstrap_started.elapsed().as_millis()).unwrap_or(u64::MAX);
            telemetry.record_bootstrap_success(
                match status.service_mode {
                    lspmux_cc_mcp::bootstrap::ServiceMode::Reused => "reused",
                    lspmux_cc_mcp::bootstrap::ServiceMode::StartedViaManager => {
                        "started_via_manager"
                    }
                    lspmux_cc_mcp::bootstrap::ServiceMode::StartedDirectly => "started_directly",
                    lspmux_cc_mcp::bootstrap::ServiceMode::Skipped => "skipped",
                },
                bootstrap_latency_ms,
            );
            tracing::info!(
                event = "bootstrap_result",
                service_mode = ?status.service_mode,
                latency_ms = bootstrap_latency_ms
            );
            status
        }
        Err(error) => {
            let bootstrap_latency_ms =
                u64::try_from(bootstrap_started.elapsed().as_millis()).unwrap_or(u64::MAX);
            telemetry.record_bootstrap_failure(
                "prepare_service",
                &error.to_string(),
                bootstrap_latency_ms,
            );
            tracing::error!(
                event = "bootstrap_result",
                outcome = "failure",
                error = %error,
                latency_ms = bootstrap_latency_ms
            );
            return Err(error).context("failed to prepare shared lspmux service");
        }
    };

    // Initialize LSP client
    let lsp = LspClient::new(
        &runtime.lspmux_path,
        &runtime.server_path,
        runtime.workspace_root.as_deref(),
    )
    .await
    .context("failed to initialize LSP client")?;

    let lsp = Arc::new(lsp);
    let tools = RustAnalyzerTools::new(Arc::clone(&lsp), runtime_status, telemetry);
    let server = LspmuxMcpServer { tools };

    // Start MCP server on stdio
    let transport = stdio();
    let service = match server.serve(transport).await {
        Ok(service) => service,
        Err(e) => {
            lsp.shutdown().await;
            return Err(e).context("failed to start MCP server");
        }
    };

    // Wait for the service to finish
    let waiting_result = service.waiting().await;

    // Gracefully shut down LSP child process
    lsp.shutdown().await;

    waiting_result.context("MCP server exited with an error")?;

    Ok(())
}
