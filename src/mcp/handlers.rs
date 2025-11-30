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

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteEntityParams {
    #[schemars(description = "Entity name to delete")]
    pub name: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteRelationParams {
    #[schemars(description = "Source entity name")]
    pub from: String,
    #[schemars(description = "Target entity name")]
    pub to: String,
    #[schemars(description = "Type of relation")]
    pub relation_type: String,
}

impl CreateEntityParams {
    fn validate(&self) -> Result<(), McpError> {
        if self.name.trim().is_empty() {
            return Err(MemoryHandler::invalid_params("Entity name cannot be empty"));
        }
        if self.entity_type.trim().is_empty() {
            return Err(MemoryHandler::invalid_params("Entity type cannot be empty"));
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

impl CreateRelationParams {
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
        params.validate()?;

        let entity = Entity::new(params.name, params.entity_type, params.observations);

        let created = self
            .storage
            .create_entity(entity)
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
        params.validate()?;

        let relation = Relation::new(params.from, params.to, params.relation_type);

        let created = self
            .storage
            .create_relation(relation)
            .await
            .map_err(Self::internal_error)?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&created)
                .unwrap_or_else(|_| "Relation created".to_string()),
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
