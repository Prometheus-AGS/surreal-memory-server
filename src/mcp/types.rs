use crate::storage::models::{Entity, Relation, SemanticSearchResult};
use serde::{Deserialize, Serialize};

// ============================================
// Request Types
// ============================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEntityRequest {
    pub name: String,
    pub entity_type: String,
    pub observations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddObservationsRequest {
    pub entity_name: String,
    pub observations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRelationRequest {
    pub from: String,
    pub to: String,
    pub relation_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchNodesRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSearchRequest {
    pub query: String,
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteEntityRequest {
    pub entity_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRelationRequest {
    pub from: String,
    pub to: String,
    pub relation_type: String,
}

// ============================================
// Response Types
// ============================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEntityResponse {
    pub success: bool,
    pub entity: Entity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddObservationsResponse {
    pub success: bool,
    pub entity: Entity,
    pub observations_added: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRelationResponse {
    pub success: bool,
    pub relation: Relation,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchNodesResponse {
    pub entities: Vec<Entity>,
    pub count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSearchResponse {
    pub results: Vec<SemanticSearchResult>,
    pub count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadGraphResponse {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
    pub entity_count: usize,
    pub relation_count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteEntityResponse {
    pub success: bool,
    pub entity_name: String,
    pub relations_deleted: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRelationResponse {
    pub success: bool,
    pub message: String,
}

// ============================================
// Helper Functions
// ============================================

fn default_limit() -> usize {
    10
}

fn default_threshold() -> f32 {
    0.7
}

// ============================================
// MCP Protocol Types
// ============================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        #[serde(rename = "data")]
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text { text: text.into() }
    }

    pub fn image(data: String, mime_type: String) -> Self {
        ContentBlock::Image { data, mime_type }
    }
}

// ============================================
// Formatting Helpers
// ============================================

impl CreateEntityResponse {
    pub fn format(&self) -> String {
        format!(
            "✅ Created entity '{}'\nType: {}\nObservations: {}\nEmbedding: {} dimensions\n{}",
            self.entity.name,
            self.entity.entity_type,
            self.entity.observations.len(),
            self.entity.embedding.as_ref().map(|e| e.len()).unwrap_or(0),
            self.message
        )
    }
}

impl AddObservationsResponse {
    pub fn format(&self) -> String {
        format!(
            "✅ Added {} observation(s) to '{}'\nTotal observations: {}\nEmbedding updated: {} dimensions\n{}",
            self.observations_added,
            self.entity.name,
            self.entity.observations.len(),
            self.entity.embedding.as_ref().map(|e| e.len()).unwrap_or(0),
            self.message
        )
    }
}

impl CreateRelationResponse {
    pub fn format(&self) -> String {
        format!(
            "✅ Created relation: {} --[{}]--> {}\n{}",
            self.relation.from, self.relation.relation_type, self.relation.to, self.message
        )
    }
}

impl SearchNodesResponse {
    pub fn format(&self) -> String {
        if self.entities.is_empty() {
            return format!("🔍 No entities found\n{}", self.message);
        }

        let mut output = format!("🔍 Found {} entity/entities:\n\n", self.count);

        for (i, entity) in self.entities.iter().enumerate() {
            output.push_str(&format!(
                "{}. {} ({})\n",
                i + 1,
                entity.name,
                entity.entity_type
            ));

            if !entity.observations.is_empty() {
                output.push_str("   Observations:\n");
                for obs in &entity.observations {
                    output.push_str(&format!("   - {}\n", obs));
                }
            }
            output.push('\n');
        }

        output.push_str(&self.message);
        output
    }
}

impl SemanticSearchResponse {
    pub fn format(&self) -> String {
        if self.results.is_empty() {
            return format!(
                "🔍 No semantically similar entities found\n{}",
                self.message
            );
        }

        let mut output = format!(
            "🔍 Found {} semantically similar entity/entities:\n\n",
            self.count
        );

        for (i, result) in self.results.iter().enumerate() {
            output.push_str(&format!(
                "{}. {} ({}) - Similarity: {:.2}%\n",
                i + 1,
                result.entity.name,
                result.entity.entity_type,
                result.similarity * 100.0
            ));

            if !result.entity.observations.is_empty() {
                output.push_str("   Observations:\n");
                for obs in &result.entity.observations {
                    output.push_str(&format!("   - {}\n", obs));
                }
            }
            output.push('\n');
        }

        output.push_str(&self.message);
        output
    }
}

impl ReadGraphResponse {
    pub fn format(&self) -> String {
        let mut output = format!(
            "📊 Knowledge Graph:\n\nEntities: {}\nRelations: {}\n\n",
            self.entity_count, self.relation_count
        );

        if !self.entities.is_empty() {
            output.push_str("=== ENTITIES ===\n\n");
            for entity in &self.entities {
                output.push_str(&format!("• {} ({})\n", entity.name, entity.entity_type));
                if !entity.observations.is_empty() {
                    for obs in &entity.observations {
                        output.push_str(&format!("  - {}\n", obs));
                    }
                }
                output.push('\n');
            }
        }

        if !self.relations.is_empty() {
            output.push_str("=== RELATIONS ===\n\n");
            for relation in &self.relations {
                output.push_str(&format!(
                    "• {} --[{}]--> {}\n",
                    relation.from, relation.relation_type, relation.to
                ));
            }
            output.push('\n');
        }

        output.push_str(&self.message);
        output
    }
}

impl DeleteEntityResponse {
    pub fn format(&self) -> String {
        format!(
            "🗑️  Deleted entity '{}'\nRelations deleted: {}\n{}",
            self.entity_name, self.relations_deleted, self.message
        )
    }
}

impl DeleteRelationResponse {
    pub fn format(&self) -> String {
        format!("🗑️  {}", self.message)
    }
}

// ============================================
// Validation Helpers
// ============================================

impl CreateEntityRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Entity name cannot be empty".to_string());
        }
        if self.entity_type.trim().is_empty() {
            return Err("Entity type cannot be empty".to_string());
        }
        if self.observations.is_empty() {
            return Err("At least one observation is required".to_string());
        }
        for obs in &self.observations {
            if obs.trim().is_empty() {
                return Err("Observations cannot be empty".to_string());
            }
        }
        Ok(())
    }
}

impl AddObservationsRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.entity_name.trim().is_empty() {
            return Err("Entity name cannot be empty".to_string());
        }
        if self.observations.is_empty() {
            return Err("At least one observation is required".to_string());
        }
        for obs in &self.observations {
            if obs.trim().is_empty() {
                return Err("Observations cannot be empty".to_string());
            }
        }
        Ok(())
    }
}

impl CreateRelationRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.from.trim().is_empty() {
            return Err("Source entity name cannot be empty".to_string());
        }
        if self.to.trim().is_empty() {
            return Err("Target entity name cannot be empty".to_string());
        }
        if self.relation_type.trim().is_empty() {
            return Err("Relation type cannot be empty".to_string());
        }
        if self.from == self.to {
            return Err("Source and target entities must be different".to_string());
        }
        Ok(())
    }
}

impl SearchNodesRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.query.trim().is_empty() {
            return Err("Search query cannot be empty".to_string());
        }
        if self.limit == 0 || self.limit > 100 {
            return Err("Limit must be between 1 and 100".to_string());
        }
        Ok(())
    }
}

impl SemanticSearchRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.query.trim().is_empty() {
            return Err("Search query cannot be empty".to_string());
        }
        if self.threshold < 0.0 || self.threshold > 1.0 {
            return Err("Threshold must be between 0.0 and 1.0".to_string());
        }
        if self.limit == 0 || self.limit > 100 {
            return Err("Limit must be between 1 and 100".to_string());
        }
        Ok(())
    }
}

impl DeleteEntityRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.entity_name.trim().is_empty() {
            return Err("Entity name cannot be empty".to_string());
        }
        Ok(())
    }
}

impl DeleteRelationRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.from.trim().is_empty() {
            return Err("Source entity name cannot be empty".to_string());
        }
        if self.to.trim().is_empty() {
            return Err("Target entity name cannot be empty".to_string());
        }
        if self.relation_type.trim().is_empty() {
            return Err("Relation type cannot be empty".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_entity_validation() {
        let valid = CreateEntityRequest {
            name: "John".to_string(),
            entity_type: "Person".to_string(),
            observations: vec!["Works at Acme".to_string()],
        };
        assert!(valid.validate().is_ok());

        let invalid_name = CreateEntityRequest {
            name: "".to_string(),
            entity_type: "Person".to_string(),
            observations: vec!["Works at Acme".to_string()],
        };
        assert!(invalid_name.validate().is_err());

        let invalid_observations = CreateEntityRequest {
            name: "John".to_string(),
            entity_type: "Person".to_string(),
            observations: vec![],
        };
        assert!(invalid_observations.validate().is_err());
    }

    #[test]
    fn test_create_relation_validation() {
        let valid = CreateRelationRequest {
            from: "John".to_string(),
            to: "Acme".to_string(),
            relation_type: "WORKS_AT".to_string(),
        };
        assert!(valid.validate().is_ok());

        let self_relation = CreateRelationRequest {
            from: "John".to_string(),
            to: "John".to_string(),
            relation_type: "KNOWS".to_string(),
        };
        assert!(self_relation.validate().is_err());
    }

    #[test]
    fn test_search_validation() {
        let valid = SearchNodesRequest {
            query: "test".to_string(),
            limit: 10,
        };
        assert!(valid.validate().is_ok());

        let invalid_limit = SearchNodesRequest {
            query: "test".to_string(),
            limit: 0,
        };
        assert!(invalid_limit.validate().is_err());

        let too_high_limit = SearchNodesRequest {
            query: "test".to_string(),
            limit: 101,
        };
        assert!(too_high_limit.validate().is_err());
    }

    #[test]
    fn test_semantic_search_validation() {
        let valid = SemanticSearchRequest {
            query: "test".to_string(),
            threshold: 0.7,
            limit: 10,
        };
        assert!(valid.validate().is_ok());

        let invalid_threshold = SemanticSearchRequest {
            query: "test".to_string(),
            threshold: 1.5,
            limit: 10,
        };
        assert!(invalid_threshold.validate().is_err());
    }
}
