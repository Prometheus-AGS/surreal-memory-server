//! Knowledge graph entity and relation types.
//!
//! Mirrors the original `models.rs` Entity/Relation schema but lives here
//! as the canonical definition for both the library and the binary server.

use serde::{Deserialize, Serialize};
use surrealdb::types::{Datetime, RecordId};
use surrealdb_types::SurrealValue;

/// A node in the knowledge graph (person, project, concept, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct Entity {
    pub id: Option<RecordId>,
    pub name: String,
    pub entity_type: String,
    pub observations: Vec<String>,
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

impl Entity {
    pub fn new(name: String, entity_type: String, observations: Vec<String>) -> Self {
        let now = Datetime::default();
        Self {
            id: None,
            name,
            entity_type,
            observations,
            embedding: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A directed edge in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct Relation {
    pub id: Option<RecordId>,
    pub from: String,
    pub to: String,
    pub relation_type: String,
    pub created_at: Datetime,
}

impl Relation {
    pub fn new(from: String, to: String, relation_type: String) -> Self {
        Self {
            id: None,
            from,
            to,
            relation_type,
            created_at: Datetime::default(),
        }
    }
}

/// The complete knowledge graph snapshot.
#[derive(Debug, Serialize, Deserialize, SurrealValue)]
pub struct KnowledgeGraph {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
}

/// An entity ranked by embedding similarity.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct SemanticSearchResult {
    pub entity: Entity,
    pub similarity: f32,
}
