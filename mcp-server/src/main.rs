//! lspmux-cc-mcp: MCP server providing rust-analyzer tools via lspmux.
//!
//! Architecture:
//! ```text
//! Claude Code <-MCP (stdio)-> lspmux-cc-mcp <-LSP (child stdio)-> lspmux client <-socket-> lspmux server -> rust-analyzer
//! ```

mod tools;

use std::sync::Arc;

use anyhow::{Context, Result};
use lspmux_cc_mcp::lsp_client::LspClient;
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
                "Provides rust-analyzer intelligence via lspmux. \
                 Use rust_diagnostics to check for errors, rust_hover for type info, \
                 rust_goto_definition to find definitions, and rust_find_references \
                 to find all usages."
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

    // Find binaries
    let cargo_home = std::env::var("CARGO_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.cargo")
    });
    let lspmux_bin = format!("{cargo_home}/bin/lspmux");

    let ra_bin = std::env::var("RUST_ANALYZER_PATH").unwrap_or_else(|_| {
        let xdg_data = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/.local/share")
        });
        format!("{xdg_data}/lspmux-rust-analyzer/current/rust-analyzer")
    });

    let workspace_root = std::env::var("WORKSPACE_ROOT").ok().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
    });

    tracing::info!("Starting lspmux-cc-mcp server");
    tracing::info!("lspmux binary: {lspmux_bin}");
    tracing::info!("rust-analyzer binary: {ra_bin}");

    // Initialize LSP client
    let lsp = LspClient::new(&lspmux_bin, &ra_bin, workspace_root.as_deref())
        .await
        .context("failed to initialize LSP client")?;

    let lsp = Arc::new(lsp);
    let tools = RustAnalyzerTools::new(Arc::clone(&lsp));
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
