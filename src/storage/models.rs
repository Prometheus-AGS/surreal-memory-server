use serde::{Deserialize, Serialize};
use surrealdb::sql::{Datetime, Thing};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: Option<Thing>,
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
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: Option<Thing>,
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
