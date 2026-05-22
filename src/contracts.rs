//! Shared REST and MCP request contracts.

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use surreal_memory::{Entity, Memory, MindMap, MindMapEdge, MindMapNode, Relation};
use surrealdb::types::RecordId;

/// Accepted mindmap map type strings.
pub const MAP_TYPES: [&str; 5] = ["radial", "concept", "argument", "tree", "temporal"];

/// Validation failure for request contracts shared across transports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError {
    message: String,
}

impl ContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ContractError {}

/// Request body for creating an entity.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateEntityRequest {
    #[schemars(description = "Unique name identifying the entity")]
    pub name: String,
    #[schemars(description = "Category or type of entity")]
    pub entity_type: String,
    #[schemars(description = "Facts or observations about the entity")]
    #[serde(deserialize_with = "crate::coerce::string_vec")]
    pub observations: Vec<String>,
}

impl CreateEntityRequest {
    /// Validate entity creation fields.
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_non_empty("Entity name", &self.name)?;
        validate_non_empty("Entity type", &self.entity_type)?;
        validate_observations(&self.observations)
    }

    /// Convert this request into the persisted domain type.
    pub fn into_entity(self) -> Entity {
        Entity::new(self.name, self.entity_type, self.observations)
    }
}

/// Request body for creating a relation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateRelationRequest {
    #[schemars(description = "Name of the source entity")]
    pub from: String,
    #[schemars(description = "Name of the target entity")]
    pub to: String,
    #[schemars(description = "Type of relationship, for example WORKS_AT")]
    pub relation_type: String,
}

impl CreateRelationRequest {
    /// Validate relation creation fields.
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_non_empty("Source entity name", &self.from)?;
        validate_non_empty("Target entity name", &self.to)?;
        validate_non_empty("Relation type", &self.relation_type)?;
        if self.from == self.to {
            return Err(ContractError::new(
                "Source and target entities must be different",
            ));
        }
        Ok(())
    }

    /// Convert this request into the persisted domain type.
    pub fn into_relation(self) -> Relation {
        Relation::new(self.from, self.to, self.relation_type)
    }
}

/// Request body for adding a scoped memory.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddMemoryRequest {
    #[schemars(description = "Content to store as a memory")]
    pub content: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    #[schemars(description = "Optional taxonomy categories")]
    #[serde(default, deserialize_with = "crate::coerce::opt_string_vec")]
    pub categories: Option<Vec<String>>,
}

impl AddMemoryRequest {
    /// Validate memory creation fields.
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_non_empty("content", &self.content)
    }

    /// Convert this request into the persisted domain type.
    pub fn into_memory(self) -> Memory {
        Memory::new(
            self.content,
            self.user_id,
            self.agent_id,
            self.session_id,
            self.categories.unwrap_or_default(),
        )
    }
}

/// Request body for updating memory content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateMemoryRequest {
    #[schemars(description = "New content for the memory")]
    pub content: String,
}

impl UpdateMemoryRequest {
    /// Validate memory update fields.
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_non_empty("content", &self.content)
    }
}

/// Request body for creating a mindmap.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateMindmapRequest {
    #[schemars(description = "Unique name for the mindmap")]
    pub name: String,
    #[schemars(description = "Map type: radial | concept | argument | tree | temporal")]
    pub map_type: Option<String>,
    #[schemars(description = "Label for the root node")]
    pub root_label: String,
    pub description: Option<String>,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
    #[schemars(description = "Optional task stream record id in the form table:key")]
    pub task_stream_id: Option<String>,
    #[serde(default, deserialize_with = "crate::coerce::opt_string_vec")]
    pub tags: Option<Vec<String>>,
}

impl CreateMindmapRequest {
    /// Validate mindmap creation fields.
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_non_empty("Mindmap name", &self.name)?;
        validate_non_empty("Root label", &self.root_label)?;
        self.map_type()?;
        Ok(())
    }

    /// Parse the requested mindmap map type, defaulting to radial.
    pub fn map_type(&self) -> Result<surreal_memory::MapType, ContractError> {
        let raw = self.map_type.as_deref().unwrap_or("radial");
        surreal_memory::MapType::parse_str(raw)
            .map_err(|_| ContractError::new(format!("invalid map_type '{}'", raw)))
    }

    /// Convert this request into the persisted domain type.
    pub fn into_mindmap(self) -> Result<MindMap, ContractError> {
        self.validate()?;
        let map_type = self.map_type()?;
        let mut mindmap = MindMap::new(
            self.name,
            map_type,
            self.root_label,
            self.description,
            self.agent_id,
            self.user_id,
        );
        if let Some(task_stream_id) = self.task_stream_id {
            mindmap.task_stream_id =
                Some(RecordId::parse_simple(&task_stream_id).map_err(|error| {
                    ContractError::new(format!("invalid task_stream_id: {error}"))
                })?);
        }
        mindmap.tags = self.tags.unwrap_or_default();
        Ok(mindmap)
    }
}

/// Request body for adding a mindmap node.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddMindmapNodeRequest {
    pub node_id: String,
    pub label: String,
    pub parent_id: Option<String>,
    #[schemars(description = "For argument maps: claim | evidence | rebuttal | idea")]
    pub node_type: Option<String>,
    pub color: Option<String>,
    #[schemars(description = "Optional arbitrary JSON metadata for the node")]
    pub metadata: Option<Value>,
}

impl AddMindmapNodeRequest {
    /// Validate node fields.
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_non_empty("node_id", &self.node_id)?;
        validate_non_empty("label", &self.label)
    }

    /// Convert this request into the persisted node type.
    pub fn into_node(self) -> MindMapNode {
        MindMapNode {
            id: self.node_id,
            label: self.label,
            parent_id: self.parent_id,
            node_type: self.node_type,
            color: self.color,
            metadata: self.metadata,
        }
    }
}

/// Request body for adding a mindmap edge.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddMindmapEdgeRequest {
    pub from_id: String,
    pub to_id: String,
    pub label: Option<String>,
    #[serde(default, deserialize_with = "crate::coerce::boolean")]
    pub directed: bool,
}

impl AddMindmapEdgeRequest {
    /// Validate edge fields.
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_non_empty("from_id", &self.from_id)?;
        validate_non_empty("to_id", &self.to_id)?;
        if self.from_id == self.to_id {
            return Err(ContractError::new(
                "Mindmap edge source and target must be different",
            ));
        }
        Ok(())
    }

    /// Convert this request into the persisted edge type.
    pub fn into_edge(self) -> MindMapEdge {
        MindMapEdge {
            from_id: self.from_id,
            to_id: self.to_id,
            label: self.label,
            directed: self.directed,
        }
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::new(format!("{field} cannot be empty")));
    }
    Ok(())
}

fn validate_observations(observations: &[String]) -> Result<(), ContractError> {
    if observations.is_empty() {
        return Err(ContractError::new("At least one observation is required"));
    }
    if observations.iter().any(|obs| obs.trim().is_empty()) {
        return Err(ContractError::new("Observations cannot be empty"));
    }
    Ok(())
}

/// Machine-readable MCP tool contract generated from shared request schemas.
pub fn mcp_tools_spec() -> Value {
    json!({
        "version": 1,
        "tools": [
            tool_spec("create_entity", "Create a new entity in the knowledge graph.", schema::<CreateEntityRequest>()),
            tool_spec("create_entities", "Create multiple entities at once.", json!({
                "type": "object",
                "required": ["entities"],
                "properties": {
                    "entities": {
                        "type": "array",
                        "items": schema::<CreateEntityRequest>()
                    }
                }
            })),
            tool_spec("create_relation", "Create a directed relationship between two existing entities.", schema::<CreateRelationRequest>()),
            tool_spec("create_relations", "Create multiple relations at once.", json!({
                "type": "object",
                "required": ["relations"],
                "properties": {
                    "relations": {
                        "type": "array",
                        "items": schema::<CreateRelationRequest>()
                    }
                }
            })),
            tool_spec("add_memory", "Add a new scoped memory.", schema::<AddMemoryRequest>()),
            tool_spec("update_memory", "Update memory content.", schema::<UpdateMemoryRequest>()),
            tool_spec("create_mindmap", "Create a mindmap.", schema::<CreateMindmapRequest>()),
            tool_spec("add_mindmap_node", "Add a node to a mindmap.", schema::<AddMindmapNodeRequest>()),
            tool_spec("add_mindmap_edge", "Add an edge to a mindmap.", schema::<AddMindmapEdgeRequest>()),
        ]
    })
}

/// Machine-readable REST contract generated from shared request schemas.
pub fn rest_api_spec() -> Value {
    json!({
        "version": 1,
        "routes": [
            route_spec("POST", "/api/v1/entities", schema::<CreateEntityRequest>()),
            route_spec("POST", "/api/v1/entities/batch", json!({
                "type": "array",
                "items": schema::<CreateEntityRequest>()
            })),
            route_spec("POST", "/api/v1/entities/relations", schema::<CreateRelationRequest>()),
            route_spec("POST", "/api/v1/entities/relations/batch", json!({
                "type": "array",
                "items": schema::<CreateRelationRequest>()
            })),
            route_spec("POST", "/api/v1/memory", schema::<AddMemoryRequest>()),
            route_spec("PUT", "/api/v1/memory/{id}", schema::<UpdateMemoryRequest>()),
            route_spec("POST", "/api/v1/mindmaps", schema::<CreateMindmapRequest>()),
            route_spec("POST", "/api/v1/mindmaps/{name}/nodes", schema::<AddMindmapNodeRequest>()),
            route_spec("POST", "/api/v1/mindmaps/{name}/edges", schema::<AddMindmapEdgeRequest>()),
        ],
        "enums": {
            "map_type": MAP_TYPES
        }
    })
}

fn tool_spec(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "input_schema": input_schema
    })
}

fn route_spec(method: &str, path: &str, request_schema: Value) -> Value {
    json!({
        "method": method,
        "path": path,
        "request_schema": request_schema
    })
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema should serialize")
}
