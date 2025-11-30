use super::{MemoryStorage, models::*};
use crate::{
    config::{Config, SurrealMode},
    embeddings::EmbeddingService,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::{cmp::Ordering, sync::Arc};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::opt::auth::Root;
use surrealdb::sql::Datetime;

pub struct SurrealStorage {
    db: Surreal<Any>,
    embedding_service: Arc<dyn EmbeddingService>,
}

impl SurrealStorage {
    pub async fn new(
        config: &Config,
        embedding_service: Arc<dyn EmbeddingService>,
    ) -> Result<Self> {
        let db = match &config.surreal_mode {
            SurrealMode::Embedded => {
                let path = config
                    .embedded_path
                    .as_ref()
                    .context("Embedded path required for embedded mode")?;

                tracing::info!("Connecting to embedded SurrealDB at: {}", path);
                surrealdb::engine::any::connect(format!("rocksdb://{}", path)).await?
            }
            SurrealMode::Server => {
                let endpoint = config
                    .surreal_endpoint
                    .as_ref()
                    .context("Endpoint required for server mode")?;

                tracing::info!("Connecting to SurrealDB server at: {}", endpoint);
                surrealdb::engine::any::connect(endpoint).await?
            }
        };

        // Sign in if credentials provided
        if let (Some(username), Some(password)) =
            (&config.surreal_username, &config.surreal_password)
        {
            db.signin(Root { username, password })
                .await
                .context("Failed to sign in to SurrealDB")?;
        }

        // Use namespace and database
        db.use_ns(&config.surreal_namespace)
            .use_db(&config.surreal_database)
            .await
            .context("Failed to use namespace/database")?;

        // Initialize schema
        Self::init_schema(&db).await?;

        Ok(Self {
            db,
            embedding_service,
        })
    }

    async fn init_schema(db: &Surreal<Any>) -> Result<()> {
        // Define entity table
        // Use array<string> and array<float> for proper array typing in SCHEMAFULL mode
        db.query(
            "DEFINE TABLE entity SCHEMAFULL;
             DEFINE FIELD name ON entity TYPE string;
             DEFINE FIELD entity_type ON entity TYPE string;
             DEFINE FIELD observations ON entity TYPE array<string>;
             DEFINE FIELD observations.* ON entity TYPE string;
             DEFINE FIELD created_at ON entity TYPE datetime;
             DEFINE FIELD updated_at ON entity TYPE datetime;
             DEFINE FIELD embedding ON entity TYPE option<array<float>>;
             DEFINE FIELD embedding.* ON entity TYPE float;
             DEFINE INDEX entity_name ON entity FIELDS name UNIQUE;",
        )
        .await?;

        // Define relation table
        db.query(
            "DEFINE TABLE relation SCHEMAFULL;
             DEFINE FIELD from ON relation TYPE string;
             DEFINE FIELD to ON relation TYPE string;
             DEFINE FIELD relation_type ON relation TYPE string;
             DEFINE FIELD created_at ON relation TYPE datetime;
             DEFINE INDEX relation_unique ON relation FIELDS from, to, relation_type UNIQUE;",
        )
        .await?;

        Ok(())
    }

    async fn compute_embedding(&self, entity: &Entity) -> Result<Vec<f32>> {
        let mut sections = vec![format!("{} ({})", entity.name, entity.entity_type)];
        if !entity.observations.is_empty() {
            sections.extend(entity.observations.iter().cloned());
        }
        let text = sections.join("\n");
        tracing::debug!(
            "Computing embedding for entity '{}' with text length {}",
            entity.name,
            text.len()
        );
        let embedding = self.embedding_service.embed(&text).await.with_context(|| {
            format!(
                "Failed to generate embedding for entity '{}': text was '{}'",
                entity.name,
                &text[..text.len().min(100)]
            )
        })?;
        tracing::debug!(
            "Successfully computed embedding for '{}' with {} dimensions",
            entity.name,
            embedding.len()
        );
        Ok(embedding)
    }

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        tracing::debug!("Embedding query: {}...", &query[..query.len().min(50)]);
        self.embedding_service.embed(query).await.with_context(|| {
            format!(
                "Failed to embed query: '{}'",
                &query[..query.len().min(100)]
            )
        })
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>();
        let norm_a = a.iter().map(|v| v * v).sum::<f32>().sqrt();
        let norm_b = b.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }
}

#[async_trait]
impl MemoryStorage for SurrealStorage {
    async fn create_entity(&self, mut entity: Entity) -> Result<Entity> {
        let now = Datetime::default();
        entity.created_at = now.clone();
        entity.updated_at = now;
        entity.embedding = Some(self.compute_embedding(&entity).await?);

        let created: Option<Entity> = self
            .db
            .create("entity")
            .content(entity)
            .await
            .context("Failed to create entity")?;

        created.context("No entity returned after creation")
    }

    async fn get_entity(&self, name: &str) -> Result<Option<Entity>> {
        let name_owned = name.to_string();
        let result: Vec<Entity> = self
            .db
            .query("SELECT * FROM entity WHERE name = $name")
            .bind(("name", name_owned))
            .await?
            .take(0)?;

        Ok(result.into_iter().next())
    }

    async fn update_entity(&self, mut entity: Entity) -> Result<Entity> {
        entity.updated_at = Datetime::default();
        entity.embedding = Some(self.compute_embedding(&entity).await?);

        let updated: Option<Entity> = self
            .db
            .query("UPDATE entity SET entity_type = $type, observations = $obs, embedding = $embedding, updated_at = $updated WHERE name = $name RETURN AFTER")
            .bind(("name", entity.name.clone()))
            .bind(("type", entity.entity_type.clone()))
            .bind(("obs", entity.observations.clone()))
            .bind(("embedding", entity.embedding.clone()))
            .bind(("updated", entity.updated_at))
            .await?
            .take(0)?;

        updated.context("Failed to update entity")
    }

    async fn delete_entity(&self, name: &str) -> Result<()> {
        let name_owned = name.to_string();
        self.db
            .query("DELETE FROM entity WHERE name = $name")
            .bind(("name", name_owned))
            .await?;
        Ok(())
    }

    async fn search_entities(&self, query: &str) -> Result<Vec<Entity>> {
        let query_owned = query.to_string();
        let results: Vec<Entity> = self
            .db
            .query("SELECT * FROM entity WHERE name CONTAINS $query OR entity_type CONTAINS $query OR observations CONTAINS $query")
            .bind(("query", query_owned))
            .await?
            .take(0)?;

        Ok(results)
    }

    async fn create_relation(&self, mut relation: Relation) -> Result<Relation> {
        relation.created_at = Datetime::default();

        let created: Option<Relation> = self
            .db
            .create("relation")
            .content(relation)
            .await
            .context("Failed to create relation")?;

        created.context("No relation returned after creation")
    }

    async fn get_relations(&self, entity_name: &str) -> Result<Vec<Relation>> {
        let name_owned = entity_name.to_string();
        let results: Vec<Relation> = self
            .db
            .query("SELECT * FROM relation WHERE from = $name OR to = $name")
            .bind(("name", name_owned))
            .await?
            .take(0)?;

        Ok(results)
    }

    async fn delete_relation(&self, from: &str, to: &str, relation_type: &str) -> Result<()> {
        let from_owned = from.to_string();
        let to_owned = to.to_string();
        let relation_owned = relation_type.to_string();
        self.db
            .query("DELETE FROM relation WHERE from = $from AND to = $to AND relation_type = $type")
            .bind(("from", from_owned))
            .bind(("to", to_owned))
            .bind(("type", relation_owned))
            .await?;
        Ok(())
    }

    async fn get_graph(&self) -> Result<KnowledgeGraph> {
        let entities: Vec<Entity> = self.db.select("entity").await?;

        let relations: Vec<Relation> = self.db.select("relation").await?;

        Ok(KnowledgeGraph {
            entities,
            relations,
        })
    }

    async fn add_observations(
        &self,
        entity_name: &str,
        observations: Vec<String>,
    ) -> Result<Entity> {
        let mut entity = self
            .get_entity(entity_name)
            .await?
            .context("Entity not found")?;
        entity.observations.extend(observations);
        self.update_entity(entity).await
    }

    async fn semantic_search(
        &self,
        query: &str,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<SemanticSearchResult>> {
        let query_embedding = self.embed_query(query).await?;
        let entities: Vec<Entity> = self
            .db
            .query("SELECT * FROM entity WHERE embedding IS NOT NONE")
            .await?
            .take(0)?;

        let mut scored: Vec<SemanticSearchResult> = Vec::new();
        for entity in entities {
            if let Some(embedding) = entity.embedding.as_ref() {
                let similarity = Self::cosine_similarity(&query_embedding, embedding);
                if similarity >= threshold {
                    scored.push(SemanticSearchResult { entity, similarity });
                }
            }
        }

        scored.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(Ordering::Equal)
        });
        if scored.len() > limit {
            scored.truncate(limit);
        }
        Ok(scored)
    }
}
