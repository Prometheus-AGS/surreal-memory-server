use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use std::sync::Arc;

use crate::storage::MemoryStorage;

pub mod handlers;

use handlers::{
    AddObservationsParams, CreateEntityParams, CreateRelationParams, DeleteEntityParams,
    DeleteRelationParams, MemoryHandler, SearchParams, SemanticSearchParams,
};

#[derive(Clone)]
pub struct MemoryMcpServer {
    handler: Arc<MemoryHandler>,
    tool_router: ToolRouter<MemoryMcpServer>,
}

impl MemoryMcpServer {
    pub fn new(storage: Arc<dyn MemoryStorage>) -> Self {
        let handler = Arc::new(MemoryHandler::new(storage));
        Self {
            handler,
            tool_router: Self::tool_router(),
        }
    }

    pub async fn run(self) -> Result<()> {
        let service = self.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }
}

#[tool_router]
impl MemoryMcpServer {
    #[tool(description = "Create a new entity")]
    async fn create_entity(
        &self,
        Parameters(params): Parameters<CreateEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_entity(params).await
    }

    #[tool(description = "Add observations to an entity")]
    async fn add_observations(
        &self,
        Parameters(params): Parameters<AddObservationsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.add_observations(params).await
    }

    #[tool(description = "Create a relation between entities")]
    async fn create_relation(
        &self,
        Parameters(params): Parameters<CreateRelationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_relation(params).await
    }

    #[tool(description = "Search entities by text query")]
    async fn search_entities(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.search(params).await
    }

    #[tool(description = "Read the full knowledge graph")]
    async fn read_graph(&self) -> Result<CallToolResult, McpError> {
        self.handler.get_graph().await
    }

    #[tool(description = "Delete an entity and its relations")]
    async fn delete_entity(
        &self,
        Parameters(params): Parameters<DeleteEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_entity(params).await
    }

    #[tool(description = "Delete a relation between entities")]
    async fn delete_relation(
        &self,
        Parameters(params): Parameters<DeleteRelationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_relation(params).await
    }

    #[tool(description = "Find semantically similar entities using embeddings")]
    async fn semantic_search(
        &self,
        Parameters(params): Parameters<SemanticSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.semantic_search(params).await
    }
}

#[tool_handler]
impl ServerHandler for MemoryMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some("Rust memory MCP server".to_string()),
            ..Default::default()
        }
    }
}
