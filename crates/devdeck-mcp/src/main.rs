//! `devdeck-mcp`: devdeck's control API as an MCP server on stdio, for Claude Code:
//! `claude mcp add devdeck -- ~/.local/bin/devdeck-mcp`. Run from a devdeck terminal, it
//! reaches that devdeck (`DEVDECK_SOCKET`) and devdeck knows it as that terminal (it's a
//! child of Claude, which runs in its shell); elsewhere, the main devdeck, as an outside
//! caller.

use devdeck_mcp::{Bridge, INSTRUCTIONS, tools};
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use std::sync::Arc;

struct Server(Arc<Bridge>);

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("devdeck", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        let bridge = self.0.clone();
        // The client blocks on devdeck's answer: off the runtime's thread.
        let methods = tokio::task::spawn_blocking(move || bridge.methods()).await.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(tools(&methods)))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let bridge = self.0.clone();
        let result = tokio::task::spawn_blocking(move || bridge.call(&request.name, request.arguments.as_ref()))
            .await
            .unwrap_or_else(|e| CallToolResult::error(vec![ContentBlock::text(format!("unavailable: {e}"))]));
        Ok(result.into())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = devdeck_api::default_socket(|k| std::env::var(k).ok());
    let service = Server(Arc::new(Bridge::new(socket))).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
