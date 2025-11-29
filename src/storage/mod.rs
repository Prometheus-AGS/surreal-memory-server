use anyhow::Result;
use async_trait::async_trait;

pub mod models;
pub mod surreal;

use models::{Entity, KnowledgeGraph, Relation, SemanticSearchResult};

#[async_trait]
pub trait MemoryStorage: Send + Sync {
    async fn create_entity(&self, entity: Entity) -> Result<Entity>;
    async fn get_entity(&self, name: &str) -> Result<Option<Entity>>;
    async fn update_entity(&self, entity: Entity) -> Result<Entity>;
    async fn delete_entity(&self, name: &str) -> Result<()>;
    async fn search_entities(&self, query: &str) -> Result<Vec<Entity>>;

    async fn create_relation(&self, relation: Relation) -> Result<Relation>;
    async fn get_relations(&self, entity_name: &str) -> Result<Vec<Relation>>;
    async fn delete_relation(&self, from: &str, to: &str, relation_type: &str) -> Result<()>;

    async fn get_graph(&self) -> Result<KnowledgeGraph>;
    async fn add_observations(
        &self,
        entity_name: &str,
        observations: Vec<String>,
    ) -> Result<Entity>;
    async fn semantic_search(
        &self,
        query: &str,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<SemanticSearchResult>>;
}
