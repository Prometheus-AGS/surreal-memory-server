use crate::{
    contracts::{
        AddMemoryRequest, AddMindmapEdgeRequest, AddMindmapNodeRequest, CreateEntityRequest,
        CreateMindmapRequest, CreateRelationRequest, UpdateMemoryRequest,
    },
    storage::MemoryStorage,
};
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

    fn internal_error(error: anyhow::Error) -> McpError {
        // Build full error chain for better debugging
        let mut error_chain = String::new();
        error_chain.push_str(&error.to_string());

        let mut source = error.source();
        while let Some(cause) = source {
            error_chain.push_str(" -> ");
            error_chain.push_str(&cause.to_string());
            source = cause.source();
        }

        tracing::error!("Internal error: {}", error_chain);
        McpError::new(ErrorCode::INTERNAL_ERROR, error_chain, None)
    }

    fn invalid_params(message: impl Into<String>) -> McpError {
        McpError::new(ErrorCode::INVALID_PARAMS, message.into(), None)
    }
}

// ── Shared param structs ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScopeFilterParams {
    #[schemars(description = "Filter by user ID")]
    pub user_id: Option<String>,
    #[schemars(description = "Filter by agent ID")]
    pub agent_id: Option<String>,
    #[schemars(description = "Filter by session ID")]
    pub session_id: Option<String>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct MemoryIdParams {
    #[schemars(description = "The full record ID of the memory (e.g. 'memory:abc123')")]
    pub id: String,
}

pub type AddMemoryParams = AddMemoryRequest;

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateMemoryParams {
    #[schemars(description = "The full record ID of the memory to update")]
    pub id: String,
    #[serde(flatten)]
    pub update: UpdateMemoryRequest,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct SearchMemoriesParams {
    #[schemars(description = "Search query")]
    pub query: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub categories: Option<Vec<String>>,
    #[schemars(description = "Maximum results to return (default 10)")]
    pub limit: Option<usize>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateTaskStreamParams {
    #[schemars(description = "Unique name for this task stream")]
    pub name: String,
    pub description: Option<String>,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
    pub model_id: Option<String>,
    pub auto_summarize: Option<bool>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskStreamNameParams {
    #[schemars(description = "Name of the task stream")]
    pub name: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddToTaskStreamParams {
    #[schemars(description = "Name of the task stream to add to")]
    pub stream_name: String,
    #[schemars(description = "Memory content to add")]
    pub content: String,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetContextParams {
    #[schemars(description = "Name of the task stream")]
    pub stream_name: String,
    #[schemars(
        description = "Model name for context budget calculation (e.g. 'gpt-4o', 'claude-3-5-sonnet')"
    )]
    pub model_name: String,
    #[schemars(description = "Override the model's default token budget")]
    pub max_tokens: Option<u64>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateEntitiesParams {
    pub entities: Vec<CreateEntityParams>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateRelationsParams {
    pub relations: Vec<CreateRelationParams>,
}

// ── Knowledge graph param structs ─────────────────────────────────────────────

pub type CreateEntityParams = CreateEntityRequest;

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddObservationsParams {
    #[schemars(description = "Name of an existing entity to add observations to")]
    pub entity_name: String,
    #[schemars(description = "New facts or observations to record about the entity")]
    pub observations: Vec<String>,
}

pub type CreateRelationParams = CreateRelationRequest;

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct EntityNameParams {
    #[schemars(description = "Exact name of the entity")]
    pub name: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct TimeParams {
    #[schemars(description = "RFC-3339 formatted timestamp (e.g., '2025-01-01T00:00:00Z')")]
    pub before_rfc3339: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    #[schemars(
        description = "Text to match against entity names and types (exact substring match)"
    )]
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
    #[schemars(
        description = "Natural language query to find similar entities (e.g., 'people who work on backend systems')"
    )]
    pub query: String,
    #[serde(default = "default_limit")]
    #[schemars(
        description = "Maximum number of results to return (1-100)",
        default = "default_limit"
    )]
    pub limit: usize,
    #[serde(default = "default_threshold")]
    #[schemars(
        description = "Minimum similarity score (0.0-1.0). Higher values return more relevant but fewer results. Default 0.5 is recommended.",
        default = "default_threshold"
    )]
    pub threshold: f32,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteEntityParams {
    #[schemars(description = "Exact name of the entity to permanently delete")]
    pub name: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteRelationParams {
    #[schemars(description = "Name of the source entity in the relation")]
    pub from: String,
    #[schemars(description = "Name of the target entity in the relation")]
    pub to: String,
    #[schemars(description = "Exact relation type to delete (e.g., 'WORKS_AT')")]
    pub relation_type: String,
}

impl AddObservationsParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.entity_name.trim().is_empty() {
            return Err(MemoryHandler::invalid_params("Entity name cannot be empty"));
        }
        if self.observations.is_empty() {
            return Err(MemoryHandler::invalid_params(
                "At least one observation is required",
            ));
        }
        if self.observations.iter().any(|obs| obs.trim().is_empty()) {
            return Err(MemoryHandler::invalid_params(
                "Observations cannot be empty",
            ));
        }
        Ok(())
    }
}

impl EntityNameParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.name.trim().is_empty() {
            return Err(MemoryHandler::invalid_params("Entity name cannot be empty"));
        }
        Ok(())
    }
}

impl TimeParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.before_rfc3339.trim().is_empty() {
            return Err(MemoryHandler::invalid_params("Timestamp cannot be empty"));
        }
        Ok(())
    }
}

impl SearchParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.query.trim().is_empty() {
            return Err(MemoryHandler::invalid_params(
                "Search query cannot be empty",
            ));
        }
        Ok(())
    }
}

impl SemanticSearchParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.query.trim().is_empty() {
            return Err(MemoryHandler::invalid_params(
                "Search query cannot be empty",
            ));
        }
        if !(0.0..=1.0).contains(&self.threshold) {
            return Err(MemoryHandler::invalid_params(
                "Threshold must be between 0.0 and 1.0",
            ));
        }
        if self.limit == 0 || self.limit > 100 {
            return Err(MemoryHandler::invalid_params(
                "Limit must be between 1 and 100",
            ));
        }
        Ok(())
    }
}

impl DeleteEntityParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.name.trim().is_empty() {
            return Err(MemoryHandler::invalid_params("Entity name cannot be empty"));
        }
        Ok(())
    }
}

impl DeleteRelationParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.from.trim().is_empty() {
            return Err(MemoryHandler::invalid_params(
                "Source entity name cannot be empty",
            ));
        }
        if self.to.trim().is_empty() {
            return Err(MemoryHandler::invalid_params(
                "Target entity name cannot be empty",
            ));
        }
        if self.relation_type.trim().is_empty() {
            return Err(MemoryHandler::invalid_params(
                "Relation type cannot be empty",
            ));
        }
        if self.from == self.to {
            return Err(MemoryHandler::invalid_params(
                "Source and target entities must be different",
            ));
        }
        Ok(())
    }
}

impl MemoryHandler {
    pub async fn create_entity(
        &self,
        params: CreateEntityParams,
    ) -> Result<CallToolResult, McpError> {
        params
            .validate()
            .map_err(|error| Self::invalid_params(error.to_string()))?;

        let created = self
            .storage
            .create_entity(params.into_entity())
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created).unwrap_or_else(|_| "Entity created".to_string()),
        )]))
    }

    pub async fn add_observations(
        &self,
        params: AddObservationsParams,
    ) -> Result<CallToolResult, McpError> {
        params.validate()?;

        let updated = self
            .storage
            .add_observations(&params.entity_name, params.observations)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&updated)
                .unwrap_or_else(|_| "Observations added".to_string()),
        )]))
    }

    pub async fn create_relation(
        &self,
        params: CreateRelationParams,
    ) -> Result<CallToolResult, McpError> {
        params
            .validate()
            .map_err(|error| Self::invalid_params(error.to_string()))?;

        let created = self
            .storage
            .create_relation(params.into_relation())
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created)
                .unwrap_or_else(|_| "Relation created".to_string()),
        )]))
    }

    pub async fn get_entity(&self, params: EntityNameParams) -> Result<CallToolResult, McpError> {
        params.validate()?;
        let entity = self
            .storage
            .get_entity(&params.name)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&entity).unwrap_or_else(|_| "null".to_string()),
        )]))
    }

    pub async fn update_entity(
        &self,
        params: CreateEntityParams,
    ) -> Result<CallToolResult, McpError> {
        params
            .validate()
            .map_err(|error| Self::invalid_params(error.to_string()))?;
        let updated = self
            .storage
            .update_entity(params.into_entity())
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&updated).unwrap_or_else(|_| "Entity updated".to_string()),
        )]))
    }

    pub async fn get_relations(
        &self,
        params: EntityNameParams,
    ) -> Result<CallToolResult, McpError> {
        params.validate()?;
        let relations = self
            .storage
            .get_relations(&params.name)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&relations).unwrap_or_else(|_| "[]".to_string()),
        )]))
    }

    pub async fn get_entity_history(
        &self,
        params: EntityNameParams,
    ) -> Result<CallToolResult, McpError> {
        params.validate()?;
        let history = self
            .storage
            .get_entity_history(&params.name)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&history).unwrap_or_else(|_| "[]".to_string()),
        )]))
    }

    pub async fn get_graph_at_time(&self, params: TimeParams) -> Result<CallToolResult, McpError> {
        params.validate()?;
        let graph = self
            .storage
            .get_graph_at_time(&params.before_rfc3339)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&graph).unwrap_or_else(|_| "{}".to_string()),
        )]))
    }

    pub async fn search(&self, params: SearchParams) -> Result<CallToolResult, McpError> {
        params.validate()?;

        let results = self
            .storage
            .search_entities(&params.query)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string()),
        )]))
    }

    pub async fn get_graph(&self) -> Result<CallToolResult, McpError> {
        let graph = self
            .storage
            .get_graph()
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&graph).unwrap_or_else(|_| "{}".to_string()),
        )]))
    }

    pub async fn semantic_search(
        &self,
        params: SemanticSearchParams,
    ) -> Result<CallToolResult, McpError> {
        params.validate()?;

        let results = self
            .storage
            .semantic_search(&params.query, params.limit, params.threshold)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string()),
        )]))
    }

    pub async fn delete_entity(
        &self,
        params: DeleteEntityParams,
    ) -> Result<CallToolResult, McpError> {
        params.validate()?;
        let name = params.name;

        let entity = self
            .storage
            .get_entity(&name)
            .await
            .map_err(Self::internal_error)?;

        if entity.is_none() {
            return Err(Self::invalid_params(format!("Entity '{}' not found", name)));
        }

        let relations = self
            .storage
            .get_relations(&name)
            .await
            .map_err(Self::internal_error)?;

        for relation in &relations {
            self.storage
                .delete_relation(&relation.from, &relation.to, &relation.relation_type)
                .await
                .map_err(Self::internal_error)?;
        }

        self.storage
            .delete_entity(&name)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted entity '{}' and removed {} relation(s)",
            name,
            relations.len()
        ))]))
    }

    pub async fn delete_relation(
        &self,
        params: DeleteRelationParams,
    ) -> Result<CallToolResult, McpError> {
        params.validate()?;

        self.storage
            .delete_relation(&params.from, &params.to, &params.relation_type)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted relation {} --[{}]--> {}",
            params.from, params.relation_type, params.to
        ))]))
    }

    // ── Batch Knowledge Graph ────────────────────────────────────────────────

    pub async fn create_entities(
        &self,
        params: CreateEntitiesParams,
    ) -> Result<CallToolResult, McpError> {
        if params.entities.is_empty() {
            return Err(Self::invalid_params("entities list cannot be empty"));
        }
        for entity in &params.entities {
            entity
                .validate()
                .map_err(|error| Self::invalid_params(error.to_string()))?;
        }
        let entities = params
            .entities
            .into_iter()
            .map(CreateEntityParams::into_entity)
            .collect();
        let created = self
            .storage
            .create_entities(entities)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created).unwrap_or_default(),
        )]))
    }

    pub async fn create_relations(
        &self,
        params: CreateRelationsParams,
    ) -> Result<CallToolResult, McpError> {
        if params.relations.is_empty() {
            return Err(Self::invalid_params("relations list cannot be empty"));
        }
        for relation in &params.relations {
            relation
                .validate()
                .map_err(|error| Self::invalid_params(error.to_string()))?;
        }
        let relations = params
            .relations
            .into_iter()
            .map(CreateRelationParams::into_relation)
            .collect();
        let created = self
            .storage
            .create_relations(relations)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created).unwrap_or_default(),
        )]))
    }

    // ── Scoped Memory ────────────────────────────────────────────────────────

    pub async fn add_memory(&self, params: AddMemoryParams) -> Result<CallToolResult, McpError> {
        params
            .validate()
            .map_err(|error| Self::invalid_params(error.to_string()))?;
        let stored = self
            .storage
            .add_memory(params.into_memory())
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&stored).unwrap_or_default(),
        )]))
    }

    pub async fn get_memory(&self, params: MemoryIdParams) -> Result<CallToolResult, McpError> {
        let mem = self
            .storage
            .get_memory(&params.id)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&mem).unwrap_or_else(|_| "null".to_string()),
        )]))
    }

    pub async fn update_memory(
        &self,
        params: UpdateMemoryParams,
    ) -> Result<CallToolResult, McpError> {
        params
            .update
            .validate()
            .map_err(|error| Self::invalid_params(error.to_string()))?;
        let updated = self
            .storage
            .update_memory(&params.id, params.update.content)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&updated).unwrap_or_default(),
        )]))
    }

    pub async fn delete_memory(&self, params: MemoryIdParams) -> Result<CallToolResult, McpError> {
        self.storage
            .delete_memory(&params.id)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Memory '{}' deleted",
            params.id
        ))]))
    }

    pub async fn delete_all_memories(
        &self,
        params: ScopeFilterParams,
    ) -> Result<CallToolResult, McpError> {
        let count = self
            .storage
            .delete_all_memories(
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                params.session_id.as_deref(),
            )
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted {} memor{}",
            count,
            if count == 1 { "y" } else { "ies" }
        ))]))
    }

    pub async fn get_all_memories(
        &self,
        params: ScopeFilterParams,
    ) -> Result<CallToolResult, McpError> {
        let memories = self
            .storage
            .get_all_memories(
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                params.session_id.as_deref(),
            )
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&memories).unwrap_or_default(),
        )]))
    }

    pub async fn search_memories(
        &self,
        params: SearchMemoriesParams,
    ) -> Result<CallToolResult, McpError> {
        if params.query.trim().is_empty() {
            return Err(Self::invalid_params("query cannot be empty"));
        }
        let results = self
            .storage
            .search_memories(
                &params.query,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                params.session_id.as_deref(),
                params.categories.as_deref(),
                params.limit.unwrap_or(10),
            )
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&results).unwrap_or_default(),
        )]))
    }

    pub async fn get_memory_history(
        &self,
        params: MemoryIdParams,
    ) -> Result<CallToolResult, McpError> {
        let history = self
            .storage
            .get_memory_history(&params.id)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&history).unwrap_or_default(),
        )]))
    }

    // ── TaskStreams ────────────────────────────────────────────────────────────

    pub async fn create_task_stream(
        &self,
        params: CreateTaskStreamParams,
    ) -> Result<CallToolResult, McpError> {
        if params.name.trim().is_empty() {
            return Err(Self::invalid_params("name cannot be empty"));
        }
        use crate::storage::TaskStream;
        let mut stream = TaskStream::new(
            params.name,
            params.description,
            params.agent_id,
            params.user_id,
        );
        stream.model_id = params.model_id;
        if let Some(auto_summarize) = params.auto_summarize {
            stream.auto_summarize = auto_summarize;
        }
        let created = self
            .storage
            .create_task_stream(stream)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created).unwrap_or_default(),
        )]))
    }

    pub async fn get_task_stream(
        &self,
        params: TaskStreamNameParams,
    ) -> Result<CallToolResult, McpError> {
        let stream = self
            .storage
            .get_task_stream(&params.name)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&stream).unwrap_or_else(|_| "null".to_string()),
        )]))
    }

    pub async fn add_to_task_stream(
        &self,
        params: AddToTaskStreamParams,
    ) -> Result<CallToolResult, McpError> {
        use crate::storage::Memory;
        let memory = Memory::new(
            params.content,
            params.user_id,
            params.agent_id,
            None,
            vec![],
        );
        let stored = self
            .storage
            .add_to_task_stream(&params.stream_name, memory)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&stored).unwrap_or_default(),
        )]))
    }

    pub async fn get_context_for_task(
        &self,
        params: GetContextParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = self
            .storage
            .get_context_for_task(&params.stream_name, &params.model_name, params.max_tokens)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&ctx).unwrap_or_default(),
        )]))
    }

    pub async fn list_task_streams(
        &self,
        params: ScopeFilterParams,
    ) -> Result<CallToolResult, McpError> {
        let streams = self
            .storage
            .list_task_streams(params.agent_id.as_deref(), params.user_id.as_deref())
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&streams).unwrap_or_default(),
        )]))
    }

    pub async fn archive_task_stream(
        &self,
        params: TaskStreamNameParams,
    ) -> Result<CallToolResult, McpError> {
        let archived = self
            .storage
            .archive_task_stream(&params.name)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&archived).unwrap_or_default(),
        )]))
    }

    pub async fn pause_task_stream(
        &self,
        params: TaskStreamNameParams,
    ) -> Result<CallToolResult, McpError> {
        let paused = self
            .storage
            .pause_task_stream(&params.name)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&paused).unwrap_or_default(),
        )]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_create_entity_params() {
        let valid = CreateEntityParams {
            name: "John".into(),
            entity_type: "Person".into(),
            observations: vec!["Works at Acme".into()],
        };
        assert!(valid.validate().is_ok());

        let missing_name = CreateEntityParams {
            name: "".into(),
            entity_type: "Person".into(),
            observations: vec!["Works at Acme".into()],
        };
        assert!(missing_name.validate().is_err());

        let empty_observation = CreateEntityParams {
            name: "John".into(),
            entity_type: "Person".into(),
            observations: vec!["".into()],
        };
        assert!(empty_observation.validate().is_err());
    }

    #[test]
    fn validate_create_relation_params() {
        let valid = CreateRelationParams {
            from: "John".into(),
            to: "Acme".into(),
            relation_type: "WORKS_AT".into(),
        };
        assert!(valid.validate().is_ok());

        let self_relation = CreateRelationParams {
            from: "John".into(),
            to: "John".into(),
            relation_type: "KNOWS".into(),
        };
        assert!(self_relation.validate().is_err());
    }

    #[test]
    fn validate_semantic_search_params() {
        let valid = SemanticSearchParams {
            query: "test".into(),
            limit: 10,
            threshold: 0.7,
        };
        assert!(valid.validate().is_ok());

        let invalid_threshold = SemanticSearchParams {
            query: "test".into(),
            limit: 10,
            threshold: 1.5,
        };
        assert!(invalid_threshold.validate().is_err());

        let invalid_limit = SemanticSearchParams {
            query: "test".into(),
            limit: 0,
            threshold: 0.5,
        };
        assert!(invalid_limit.validate().is_err());
    }
}

// ── Mindmap param structs ────────────────────────────────────────────────────

pub type CreateMindmapParams = CreateMindmapRequest;

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct MindmapNameParams {
    pub name: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddMindmapNodeParams {
    pub mindmap_name: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    #[schemars(description = "Unique node id within this mindmap")]
    pub node_id: String,
    pub label: String,
    pub parent_id: Option<String>,
    #[schemars(description = "For argument maps: claim | evidence | rebuttal | idea")]
    pub node_type: Option<String>,
    pub color: Option<String>,
    #[schemars(description = "Optional arbitrary JSON metadata for the node")]
    pub metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddMindmapEdgeParams {
    pub mindmap_name: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub from_id: String,
    pub to_id: String,
    pub label: Option<String>,
    #[serde(default)]
    pub directed: bool,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteMindmapNodeParams {
    pub mindmap_name: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub node_id: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExportMindmapParams {
    pub name: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    #[schemars(description = "Export format: json | mermaid | markdown")]
    pub format: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct GeneratePersonaMindmapParams {
    pub user_id: String,
    #[schemars(description = "Name for the generated mindmap")]
    pub name: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct GenerateIdeationMindmapParams {
    pub topic: String,
    #[schemars(description = "Map type: radial | concept | argument | tree | temporal")]
    pub map_type: String,
    pub context: Option<String>,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
}

// ── Hybrid search + advanced mem0 param structs ──────────────────────────────

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct HybridSearchParams {
    pub query: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    #[serde(default = "hybrid_default_limit")]
    pub limit: usize,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    #[serde(default = "default_bm25_weight")]
    pub bm25_weight: f32,
}
fn default_vector_weight() -> f32 {
    0.7
}
fn default_bm25_weight() -> f32 {
    0.3
}
fn hybrid_default_limit() -> usize {
    10
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompressMemoriesParams {
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    #[schemars(description = "Compress memories older than this many days")]
    pub older_than_days: u32,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConversationMessage {
    #[schemars(description = "Role of the speaker: 'user' or 'assistant'")]
    pub role: String,
    #[schemars(description = "Text content of the message")]
    pub content: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConversationParams {
    #[schemars(description = "Array of role+content message objects")]
    pub messages: Vec<ConversationMessage>,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListMindmapsParams {
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
}

// ── impl MemoryHandler — new methods ────────────────────────────────────────

impl MemoryHandler {
    // Mindmap handlers

    pub async fn create_mindmap(
        &self,
        params: CreateMindmapParams,
    ) -> Result<CallToolResult, McpError> {
        let mm = params
            .into_mindmap()
            .map_err(|error| Self::invalid_params(error.to_string()))?;
        match self.storage.create_mindmap(mm).await {
            Ok(created) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&created).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn get_mindmap(&self, params: MindmapNameParams) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .get_mindmap(
                &params.name,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
            )
            .await
        {
            Ok(Some(mm)) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&mm).unwrap_or_default(),
            )])),
            Ok(None) => Ok(CallToolResult::success(vec![Content::text("null")])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn list_mindmaps(
        &self,
        params: ListMindmapsParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .list_mindmaps(params.user_id.as_deref(), params.agent_id.as_deref())
            .await
        {
            Ok(maps) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&maps).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn add_mindmap_node(
        &self,
        params: AddMindmapNodeParams,
    ) -> Result<CallToolResult, McpError> {
        let node_request = AddMindmapNodeRequest {
            node_id: params.node_id,
            label: params.label,
            parent_id: params.parent_id,
            node_type: params.node_type,
            color: params.color,
            metadata: params
                .metadata
                .map(|m| serde_json::Value::Object(m.into_iter().collect())),
        };
        node_request
            .validate()
            .map_err(|error| Self::invalid_params(error.to_string()))?;
        match self
            .storage
            .add_mindmap_node(
                &params.mindmap_name,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                node_request.into_node(),
            )
            .await
        {
            Ok(mm) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&mm).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn add_mindmap_edge(
        &self,
        params: AddMindmapEdgeParams,
    ) -> Result<CallToolResult, McpError> {
        let edge_request = AddMindmapEdgeRequest {
            from_id: params.from_id,
            to_id: params.to_id,
            label: params.label,
            directed: params.directed,
        };
        edge_request
            .validate()
            .map_err(|error| Self::invalid_params(error.to_string()))?;
        match self
            .storage
            .add_mindmap_edge(
                &params.mindmap_name,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                edge_request.into_edge(),
            )
            .await
        {
            Ok(mm) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&mm).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn delete_mindmap_node(
        &self,
        params: DeleteMindmapNodeParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .delete_mindmap_node(
                &params.mindmap_name,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                &params.node_id,
            )
            .await
        {
            Ok(mm) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&mm).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn delete_mindmap(
        &self,
        params: MindmapNameParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .delete_mindmap(
                &params.name,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
            )
            .await
        {
            Ok(()) => Ok(CallToolResult::success(vec![Content::text(
                "Mindmap deleted",
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn export_mindmap(
        &self,
        params: ExportMindmapParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::ExportFormat;
        let fmt = match params.format.to_lowercase().as_str() {
            "mermaid" => ExportFormat::Mermaid,
            "markdown" | "md" => ExportFormat::Markdown,
            _ => ExportFormat::Json,
        };
        match self
            .storage
            .get_mindmap(
                &params.name,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
            )
            .await
        {
            Ok(Some(mm)) => Ok(CallToolResult::success(vec![Content::text(
                mm.export(&fmt),
            )])),
            Ok(None) => Err(Self::invalid_params(format!(
                "Mindmap '{}' not found",
                params.name
            ))),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn generate_persona_mindmap(
        &self,
        params: GeneratePersonaMindmapParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::{MapType, MindMap, MindMapNode};
        // Fetch all memories for this user and cluster by category
        let memories = self
            .storage
            .get_all_memories(Some(&params.user_id), None, None)
            .await
            .map_err(Self::internal_error)?;

        let mut mm = MindMap::new(
            params.name.clone(),
            MapType::Radial,
            format!("Persona: {}", params.user_id),
            Some(format!("Auto-generated from {} memories", memories.len())),
            None,
            Some(params.user_id.clone()),
        );

        // Cluster memories by first category tag, default to "general"
        use std::collections::HashMap;
        let mut clusters: HashMap<String, Vec<String>> = HashMap::new();
        for mem in &memories {
            let cat = mem
                .categories
                .first()
                .cloned()
                .unwrap_or_else(|| "general".to_string());
            clusters.entry(cat).or_default().push(mem.content.clone());
        }

        for (cat, items) in &clusters {
            let branch_id = cat.replace(' ', "_");
            mm.nodes.push(MindMapNode {
                id: branch_id.clone(),
                label: cat.clone(),
                parent_id: Some("root".to_string()),
                node_type: Some("branch".to_string()),
                color: None,
                metadata: None,
            });
            for (i, item) in items.iter().take(5).enumerate() {
                let leaf_id = format!("{}_leaf_{}", branch_id, i);
                let short = if item.len() > 80 {
                    format!("{}…", &item[..80])
                } else {
                    item.clone()
                };
                mm.nodes.push(MindMapNode {
                    id: leaf_id,
                    label: short,
                    parent_id: Some(branch_id.clone()),
                    node_type: Some("leaf".to_string()),
                    color: None,
                    metadata: None,
                });
            }
        }

        match self.storage.create_mindmap(mm).await {
            Ok(created) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&created).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn generate_ideation_mindmap(
        &self,
        params: GenerateIdeationMindmapParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::{MapType, MindMap};
        let map_type = match params.map_type.to_lowercase().as_str() {
            "concept" => MapType::Concept,
            "argument" => MapType::Argument,
            "tree" => MapType::Tree,
            "temporal" => MapType::Temporal,
            _ => MapType::Radial,
        };
        let mm = MindMap::new(
            format!("ideation_{}", params.topic.replace(' ', "_").to_lowercase()),
            map_type,
            params.topic.clone(),
            params.context,
            params.agent_id,
            params.user_id,
        );
        match self.storage.create_mindmap(mm).await {
            Ok(created) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&created).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    // Hybrid search + mem0 advanced handlers

    pub async fn hybrid_search_memories(
        &self,
        params: HybridSearchParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .hybrid_search_memories(
                &params.query,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                params.session_id.as_deref(),
                params.limit,
                params.vector_weight,
                params.bm25_weight,
            )
            .await
        {
            Ok(mems) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&mems).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn compress_memories(
        &self,
        params: CompressMemoriesParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .compress_memories(
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                params.session_id.as_deref(),
                params.older_than_days,
            )
            .await
        {
            Ok(Some(summary)) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&summary).unwrap_or_default(),
            )])),
            Ok(None) => Ok(CallToolResult::success(vec![Content::text(
                "No memories to compress",
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn add_memories_from_conversation(
        &self,
        params: ConversationParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .add_memories_from_conversation(
                params
                    .messages
                    .into_iter()
                    .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
                    .collect(),
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                params.session_id.as_deref(),
            )
            .await
        {
            Ok(mems) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Stored {} memories from conversation",
                mems.len()
            ))])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    // ── Phase 3: Advanced Context + Graph-RAG ────────────────────────────────

    pub async fn auto_summarize_task_stream(
        &self,
        params: AutoSummarizeTaskStreamParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .auto_summarize_task_stream(
                &params.stream_name,
                params.user_id.as_deref(),
                params.agent_id.as_deref(),
                params.model_id.as_deref().unwrap_or("default"),
            )
            .await
        {
            Ok(Some(m)) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Auto-summarized task stream '{}'. Summary: {}",
                params.stream_name, m.content
            ))])),
            Ok(None) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Task stream '{}' does not need summarization yet.",
                params.stream_name
            ))])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn find_path(&self, params: FindPathParams) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .find_path(&params.from, &params.to, params.max_depth.unwrap_or(4))
            .await
        {
            Ok(paths) if paths.is_empty() => Ok(CallToolResult::success(vec![Content::text(
                format!("No path found from '{}' to '{}'", params.from, params.to),
            )])),
            Ok(paths) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&paths).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn expand_neighbors(
        &self,
        params: ExpandNeighborsParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .expand_neighbors(
                &params.entity_name,
                params.depth.unwrap_or(2),
                params.limit.unwrap_or(50),
            )
            .await
        {
            Ok(graph) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&graph).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }

    pub async fn get_related(&self, params: GetRelatedParams) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .get_related(
                &params.entity_name,
                params.relation_type.as_deref(),
                params.direction.as_deref().unwrap_or("both"),
                params.limit.unwrap_or(20),
            )
            .await
        {
            Ok(entities) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&entities).unwrap_or_default(),
            )])),
            Err(e) => Err(Self::internal_error(e)),
        }
    }
}

// ── Phase 3 param structs ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct AutoSummarizeTaskStreamParams {
    #[schemars(description = "Name of the task stream to attempt summarization on")]
    pub stream_name: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    #[schemars(
        description = "Model profile ID for budget calculation (e.g. gpt-4o, claude-3-5-sonnet)"
    )]
    pub model_id: Option<String>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct FindPathParams {
    #[schemars(description = "Name of the source entity")]
    pub from: String,
    #[schemars(description = "Name of the destination entity")]
    pub to: String,
    #[schemars(description = "Maximum hops to traverse (default 4, max 6)")]
    pub max_depth: Option<u8>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExpandNeighborsParams {
    #[schemars(description = "Name of the entity to expand from")]
    pub entity_name: String,
    #[schemars(description = "Number of hops to expand (default 2, max 5)")]
    pub depth: Option<u8>,
    #[schemars(description = "Maximum total entities to return (default 50)")]
    pub limit: Option<usize>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetRelatedParams {
    #[schemars(description = "Name of the entity to find relations for")]
    pub entity_name: String,
    #[schemars(description = "Filter by relation type (e.g. WORKS_AT, KNOWS)")]
    pub relation_type: Option<String>,
    #[schemars(description = "Direction: in | out | both (default: both)")]
    pub direction: Option<String>,
    #[schemars(description = "Maximum results (default 20)")]
    pub limit: Option<usize>,
}

// ── Palace param structs ──────────────────────────────────────────────────────
// Always compiled so the #[tool_router] macro can reference them unconditionally.

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceWakeUpParams {
    #[schemars(description = "Optional wing to scope the wake-up context")]
    pub wing: Option<String>,
    #[schemars(description = "If true, compress output using AAAK Dialect")]
    pub compress: Option<bool>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceRecallParams {
    #[schemars(description = "Optional wing to filter by")]
    pub wing: Option<String>,
    #[schemars(description = "Optional room to filter by")]
    pub room: Option<String>,
    #[schemars(description = "Maximum drawers to recall (default: 20)")]
    pub limit: Option<u32>,
    #[schemars(description = "If true, compress output using AAAK Dialect")]
    pub compress: Option<bool>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceSearchParams {
    #[schemars(description = "Natural language search query")]
    pub query: String,
    #[schemars(description = "Optional wing to filter by")]
    pub wing: Option<String>,
    #[schemars(description = "Optional room to filter by")]
    pub room: Option<String>,
    #[schemars(description = "Number of results (default: 10)")]
    pub n: Option<u32>,
    #[schemars(description = "If true, compress output using AAAK Dialect")]
    pub compress: Option<bool>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceIngestParams {
    #[schemars(description = "Content to store")]
    pub content: String,
    #[schemars(description = "Wing (top-level category)")]
    pub wing: String,
    #[schemars(description = "Room within the wing")]
    pub room: String,
    #[schemars(description = "Hall within the room (default: 'general')")]
    pub hall: Option<String>,
    #[schemars(description = "Importance 0.0-1.0 (default: 1.0)")]
    pub importance: Option<f32>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceDeleteParams {
    #[schemars(description = "Drawer record ID (e.g. 'drawers:abc123')")]
    pub id: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct PalaceHybridSearchParams {
    #[schemars(description = "Natural language search query")]
    pub query: String,
    #[schemars(description = "Memory scope filter ('global', 'agent', 'user', 'session')")]
    pub scope: Option<String>,
    #[schemars(description = "Optional wing to filter palace drawers")]
    pub wing: Option<String>,
    #[schemars(description = "Number of results per source (default: 10)")]
    pub n: Option<u32>,
    #[schemars(description = "If true, compress content using AAAK Dialect")]
    pub compress: Option<bool>,
}

// ── Palace handler methods ────────────────────────────────────────────────────

#[cfg(feature = "palace")]
impl MemoryHandler {
    fn get_palace_storage(&self) -> Result<&surreal_memory::SurrealStorage, McpError> {
        self.storage
            .as_any()
            .downcast_ref::<surreal_memory::SurrealStorage>()
            .ok_or_else(|| {
                Self::internal_error(anyhow::anyhow!(
                    "Palace features require SurrealStorage backend"
                ))
            })
    }

    pub async fn palace_wake_up(
        &self,
        params: PalaceWakeUpParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self.get_palace_storage()?;
        let result = storage
            .palace_wake_up(params.wing.as_deref())
            .await
            .map_err(Self::internal_error)?;
        let output = if params.compress.unwrap_or(false) {
            storage.palace_compress(&result)
        } else {
            result
        };
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    pub async fn palace_recall(
        &self,
        params: PalaceRecallParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self.get_palace_storage()?;
        let limit = params.limit.unwrap_or(20) as usize;
        let result = storage
            .palace_recall(params.wing.as_deref(), params.room.as_deref(), limit)
            .await
            .map_err(Self::internal_error)?;
        let output = if params.compress.unwrap_or(false) {
            storage.palace_compress(&result)
        } else {
            result
        };
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    pub async fn palace_search(
        &self,
        params: PalaceSearchParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        if params.query.trim().is_empty() {
            return Err(Self::invalid_params("query cannot be empty"));
        }
        let storage = self.get_palace_storage()?;
        let n = params.n.unwrap_or(10) as usize;
        let result = storage
            .palace_search(
                &params.query,
                params.wing.as_deref(),
                params.room.as_deref(),
                n,
            )
            .await
            .map_err(Self::internal_error)?;
        let output = if params.compress.unwrap_or(false) {
            storage.palace_compress(&result)
        } else {
            result
        };
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    pub async fn palace_ingest(
        &self,
        params: PalaceIngestParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        if params.content.trim().is_empty() {
            return Err(Self::invalid_params("content cannot be empty"));
        }
        if params.wing.trim().is_empty() {
            return Err(Self::invalid_params("wing cannot be empty"));
        }
        if params.room.trim().is_empty() {
            return Err(Self::invalid_params("room cannot be empty"));
        }
        let storage = self.get_palace_storage()?;
        let hall = params.hall.as_deref().unwrap_or("general");
        let importance = params.importance.unwrap_or(1.0);
        let drawer_id = storage
            .palace_ingest(
                &params.content,
                &params.wing,
                &params.room,
                hall,
                importance,
            )
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({ "drawer_id": drawer_id }).to_string(),
        )]))
    }

    pub async fn palace_delete(
        &self,
        params: PalaceDeleteParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        if params.id.trim().is_empty() {
            return Err(Self::invalid_params("id cannot be empty"));
        }
        let storage = self.get_palace_storage()?;
        storage
            .palace_delete(&params.id)
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({ "deleted": true, "id": params.id }).to_string(),
        )]))
    }

    pub async fn palace_status(&self) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        let storage = self.get_palace_storage()?;
        let status = storage
            .palace_status()
            .await
            .map_err(Self::internal_error)?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&status).unwrap_or_default(),
        )]))
    }

    pub async fn palace_hybrid_search(
        &self,
        params: PalaceHybridSearchParams,
    ) -> Result<CallToolResult, McpError> {
        use surreal_memory::PalaceStorage;
        if params.query.trim().is_empty() {
            return Err(Self::invalid_params("query cannot be empty"));
        }
        let storage = self.get_palace_storage()?;
        let scope = params.scope.as_deref().and_then(|s| match s {
            "global" => Some(surreal_memory::MemoryScope::Global),
            "agent" => Some(surreal_memory::MemoryScope::Agent),
            "user" => Some(surreal_memory::MemoryScope::User),
            "session" => Some(surreal_memory::MemoryScope::Session),
            _ => None,
        });
        let n = params.n.unwrap_or(10) as usize;
        let hits = storage
            .palace_hybrid_search(&params.query, scope, params.wing.as_deref(), n)
            .await
            .map_err(Self::internal_error)?;
        let output = if params.compress.unwrap_or(false) {
            let json = serde_json::to_string_pretty(&hits).unwrap_or_default();
            storage.palace_compress(&json)
        } else {
            serde_json::to_string_pretty(&hits).unwrap_or_default()
        };
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

/// Fallback palace handlers when the `palace` feature is not enabled.
/// Returns an error explaining the feature is not compiled in.
#[cfg(not(feature = "palace"))]
impl MemoryHandler {
    fn palace_not_enabled() -> McpError {
        McpError::new(
            ErrorCode::INTERNAL_ERROR,
            "Palace features are not enabled. Rebuild with --features palace".to_string(),
            None,
        )
    }

    pub async fn palace_wake_up(
        &self,
        _params: PalaceWakeUpParams,
    ) -> Result<CallToolResult, McpError> {
        Err(Self::palace_not_enabled())
    }

    pub async fn palace_recall(
        &self,
        _params: PalaceRecallParams,
    ) -> Result<CallToolResult, McpError> {
        Err(Self::palace_not_enabled())
    }

    pub async fn palace_search(
        &self,
        _params: PalaceSearchParams,
    ) -> Result<CallToolResult, McpError> {
        Err(Self::palace_not_enabled())
    }

    pub async fn palace_ingest(
        &self,
        _params: PalaceIngestParams,
    ) -> Result<CallToolResult, McpError> {
        Err(Self::palace_not_enabled())
    }

    pub async fn palace_delete(
        &self,
        _params: PalaceDeleteParams,
    ) -> Result<CallToolResult, McpError> {
        Err(Self::palace_not_enabled())
    }

    pub async fn palace_status(&self) -> Result<CallToolResult, McpError> {
        Err(Self::palace_not_enabled())
    }

    pub async fn palace_hybrid_search(
        &self,
        _params: PalaceHybridSearchParams,
    ) -> Result<CallToolResult, McpError> {
        Err(Self::palace_not_enabled())
    }
}
