use crate::storage::{MemoryStorage, models::*};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, Content, ErrorCode},
    schemars,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct MemoryHandler {
    storage: Arc<dyn MemoryStorage>,
}

impl MemoryHandler {
    pub fn new(storage: Arc<dyn MemoryStorage>) -> Self {
        Self { storage }
    }

    fn internal_error(error: impl std::fmt::Display) -> McpError {
        McpError::new(ErrorCode::INTERNAL_ERROR, error.to_string(), None)
    }
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateEntityParams {
    #[schemars(description = "Name of the entity")]
    pub name: String,
    #[schemars(description = "Type of the entity")]
    pub entity_type: String,
    #[schemars(description = "Initial observations about the entity")]
    pub observations: Vec<String>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddObservationsParams {
    #[schemars(description = "Entity name")]
    pub entity_name: String,
    #[schemars(description = "Observations to add")]
    pub observations: Vec<String>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateRelationParams {
    #[schemars(description = "Source entity name")]
    pub from: String,
    #[schemars(description = "Target entity name")]
    pub to: String,
    #[schemars(description = "Type of relation")]
    pub relation_type: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    #[schemars(description = "Search query")]
    pub query: String,
}

fn default_limit() -> usize {
    10
}

fn default_threshold() -> f32 {
    0.5
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct SemanticSearchParams {
    #[schemars(description = "Query text to embed")]
    pub query: String,
    #[serde(default = "default_limit")]
    #[schemars(description = "Maximum number of results", default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_threshold")]
    #[schemars(
        description = "Similarity threshold between 0 and 1",
        default = "default_threshold"
    )]
    pub threshold: f32,
}

impl MemoryHandler {
    pub async fn create_entity(
        &self,
        params: CreateEntityParams,
    ) -> Result<CallToolResult, McpError> {
        let entity = Entity {
            id: None,
            name: params.name,
            entity_type: params.entity_type,
            observations: params.observations,
            embedding: None,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let created = self
            .storage
            .create_entity(entity)
            .await
            .map_err(|e| Self::internal_error(e))?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created).unwrap_or_else(|_| "Entity created".to_string()),
        )]))
    }

    pub async fn add_observations(
        &self,
        params: AddObservationsParams,
    ) -> Result<CallToolResult, McpError> {
        let updated = self
            .storage
            .add_observations(&params.entity_name, params.observations)
            .await
            .map_err(|e| Self::internal_error(e))?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&updated)
                .unwrap_or_else(|_| "Observations added".to_string()),
        )]))
    }

    pub async fn create_relation(
        &self,
        params: CreateRelationParams,
    ) -> Result<CallToolResult, McpError> {
        let relation = Relation {
            id: None,
            from: params.from,
            to: params.to,
            relation_type: params.relation_type,
            created_at: String::new(),
        };

        let created = self
            .storage
            .create_relation(relation)
            .await
            .map_err(|e| Self::internal_error(e))?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created)
                .unwrap_or_else(|_| "Relation created".to_string()),
        )]))
    }

    pub async fn search(&self, params: SearchParams) -> Result<CallToolResult, McpError> {
        let results = self
            .storage
            .search_entities(&params.query)
            .await
            .map_err(|e| Self::internal_error(e))?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string()),
        )]))
    }

    pub async fn get_graph(&self) -> Result<CallToolResult, McpError> {
        let graph = self
            .storage
            .get_graph()
            .await
            .map_err(|e| Self::internal_error(e))?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&graph).unwrap_or_else(|_| "{}".to_string()),
        )]))
    }

    pub async fn semantic_search(
        &self,
        params: SemanticSearchParams,
    ) -> Result<CallToolResult, McpError> {
        let results = self
            .storage
            .semantic_search(&params.query, params.limit, params.threshold)
            .await
            .map_err(|e| Self::internal_error(e))?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string()),
        )]))
    }
}
