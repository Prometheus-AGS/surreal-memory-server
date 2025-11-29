use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: Option<Thing>,
    pub name: String,
    pub entity_type: String,
    pub observations: Vec<String>,
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: Option<Thing>,
    pub from: String,
    pub to: String,
    pub relation_type: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSearchResult {
    pub entity: Entity,
    pub similarity: f32,
}
