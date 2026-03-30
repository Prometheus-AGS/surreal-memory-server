//! SurrealDB-backed implementation of `MemoryStorage`.

use super::MemoryStorage;
use crate::{
    embeddings::EmbeddingService,
    entity::{Entity, KnowledgeGraph, Relation, SemanticSearchResult},
    memory::{Memory, MemoryHistory},
    mindmap::MindMap,
    storage::migrations::run_migrations,
    task_stream::{ContextWindow, TaskStream, TaskStreamStatus},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use std::{cmp::Ordering, sync::Arc, sync::RwLock};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::opt::auth::Root;
use surrealdb::types::Datetime;
use surrealdb_types::SurrealValue;
use uuid::Uuid;

/// Token budget constants per model family. Extend via config in Phase 3.
const DEFAULT_CONTEXT_BUDGET: u64 = 100_000;

pub struct SurrealStorage {
    connection: Arc<RwLock<ConnectionState>>,
    connection_info: ConnectionInfo,
    embedding_service: Arc<dyn EmbeddingService>,
}

// ── Retry Configuration ───────────────────────────────────────────────────────

use std::time::Duration;
use rand::Rng;

/// Configuration for retry and reconnection behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_connect_retries: u32,
    pub max_operation_retries: u32,
    pub base_retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
    pub jitter_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_connect_retries: 10,
            max_operation_retries: 3,
            base_retry_delay_ms: 100,
            max_retry_delay_ms: 5000,
            jitter_factor: 0.25,
        }
    }
}

impl RetryConfig {
    /// Calculate exponential backoff delay with jitter.
    /// Formula: min(base * 2^attempt, max) * (1 ± jitter_factor)
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let mut rng = rand::thread_rng();

        // Exponential backoff: base * 2^attempt
        let base_delay = self.base_retry_delay_ms.saturating_mul(2u64.saturating_pow(attempt));

        // Apply max cap
        let capped_delay = base_delay.min(self.max_retry_delay_ms);

        // Apply jitter: delay * (1 ± jitter_factor)
        let jitter_range = (capped_delay as f64 * self.jitter_factor) as u64;
        let min_delay = capped_delay.saturating_sub(jitter_range);
        let max_delay = capped_delay.saturating_add(jitter_range);

        let jittered_delay = rng.gen_range(min_delay..=max_delay);
        Duration::from_millis(jittered_delay)
    }
}

/// Connection state for health tracking and reconnection.
#[derive(Debug)]
enum ConnectionState {
    Connected(Surreal<Any>),
    Reconnecting,
    Failed(String),
}

/// Connection configuration for reconnection attempts.
#[derive(Debug, Clone)]
struct ConnectionInfo {
    config: SurrealConfig,
    retry_config: RetryConfig,
}

// ── Config-compatible constructor ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SurrealMode {
    Embedded,
    Server,
}

#[derive(Debug, Clone)]
pub struct SurrealConfig {
    pub mode: SurrealMode,
    pub endpoint: Option<String>,
    pub embedded_path: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub namespace: String,
    pub database: String,
    pub retry: RetryConfig,
}

impl Default for SurrealConfig {
    fn default() -> Self {
        Self {
            mode: SurrealMode::Embedded,
            endpoint: None,
            embedded_path: None,
            username: None,
            password: None,
            namespace: "default".to_string(),
            database: "default".to_string(),
            retry: RetryConfig::default(),
        }
    }
}

impl SurrealStorage {
    pub async fn new(
        config: &SurrealConfig,
        embedding_service: Arc<dyn EmbeddingService>,
    ) -> Result<Self> {
        let connection_info = ConnectionInfo {
            config: config.clone(),
            retry_config: config.retry.clone(),
        };

        let db = Self::connect_with_config(config).await?;

        // Run migrations on initial connection
        run_migrations(&db).await?;

        let connection = Arc::new(RwLock::new(ConnectionState::Connected(db)));

        Ok(Self {
            connection,
            connection_info,
            embedding_service,
        })
    }

    /// Establish connection without retry logic (called by connect_with_retry).
    async fn connect_with_config(config: &SurrealConfig) -> Result<Surreal<Any>> {
        let db = match &config.mode {
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
                    .endpoint
                    .as_ref()
                    .context("Endpoint required for server mode")?;
                tracing::info!("Connecting to SurrealDB server at: {}", endpoint);
                surrealdb::engine::any::connect(endpoint).await?
            }
        };

        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            db.signin(Root {
                username: username.clone(),
                password: password.clone(),
            })
            .await
            .context("Failed to sign in to SurrealDB")?;
        }

        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .context("Failed to use namespace/database")?;

        Ok(db)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    async fn embed_entity(&self, entity: &Entity) -> Result<Vec<f32>> {
        let mut parts = vec![format!("{} ({})", entity.name, entity.entity_type)];
        parts.extend(entity.observations.iter().cloned());
        self.embedding_service.embed(&parts.join("\n")).await
    }

    async fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        self.embedding_service.embed(text).await
    }

    async fn create_record<T>(&self, table: &str, key: &str, value: T, op: &str) -> Result<T>
    where
        T: Serialize + DeserializeOwned + SurrealValue,
    {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut response = db
            .query("CREATE type::record($table, $key) CONTENT $value RETURN AFTER")
            .bind(("table", table.to_string()))
            .bind(("key", key.to_string()))
            .bind(("value", value))
            .await
            .with_context(|| format!("{op}: SurrealDB create query failed"))?;
        let created: Option<T> = response
            .take(0)
            .with_context(|| format!("{op}: SurrealDB rejected the write"))?;
        created.with_context(|| format!("{op}: SurrealDB returned no record after write"))
    }

    async fn replace_record<T>(&self, record_id: &str, value: T, op: &str) -> Result<T>
    where
        T: Serialize + DeserializeOwned + SurrealValue,
    {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut response = db
            .query("UPDATE $id CONTENT $value RETURN AFTER")
            .bind(("id", record_id.to_string()))
            .bind(("value", value))
            .await
            .with_context(|| format!("{op}: SurrealDB update query failed"))?;
        let updated: Option<T> = response
            .take(0)
            .with_context(|| format!("{op}: SurrealDB rejected the write"))?;
        updated.with_context(|| format!("{op}: SurrealDB returned no record after write"))
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|v| v * v).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }

    /// Convert a `RecordId` to its canonical `table:key` string.
    /// Works without requiring `Display` on `RecordIdKey`.
    fn record_id_to_string(id: &surrealdb::types::RecordId) -> String {
        use surrealdb::types::RecordIdKey;
        let key_str = match &id.key {
            RecordIdKey::String(s) => s.clone(),
            RecordIdKey::Number(i) => i.to_string(),
            k => format!("{k:?}"),
        };
        format!("{}:{}", id.table.as_str(), key_str)
    }

    fn estimate_tokens(text: &str) -> u32 {
        // Rough heuristic: ~4 chars per token (good enough for budget tracking)
        (text.len() as u32 + 3) / 4
    }

    fn model_context_budget(model_name: &str) -> u64 {
        match model_name {
            m if m.starts_with("gpt-4o") => 120_000,
            m if m.starts_with("gpt-4") => 120_000,
            m if m.starts_with("claude-3") => 180_000,
            m if m.starts_with("gemini-2") => 900_000,
            m if m.starts_with("gemini-1.5") => 900_000,
            _ => DEFAULT_CONTEXT_BUDGET,
        }
    }
}

// ── MemoryStorage impl ────────────────────────────────────────────────────────

#[async_trait]
impl MemoryStorage for SurrealStorage {
    // ── Knowledge Graph ───────────────────────────────────────────────────────

    async fn create_entity(&self, mut entity: Entity) -> Result<Entity> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let now = Datetime::default();
        entity.created_at = now.clone();
        entity.updated_at = now;
        entity.embedding = Some(self.embed_entity(&entity).await?);

        let created: Option<Entity> = db
            .create("entity")
            .content(entity)
            .await
            .context("Failed to create entity")?;

        created.ok_or_else(|| anyhow::anyhow!("No entity returned after creation"))
    }

    async fn create_entities(&self, entities: Vec<Entity>) -> Result<Vec<Entity>> {
        let mut results = Vec::with_capacity(entities.len());
        for entity in entities {
            results.push(self.create_entity(entity).await?);
        }
        Ok(results)
    }

    async fn get_entity(&self, name: &str) -> Result<Option<Entity>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let result: Vec<Entity> = db
            .query("SELECT * FROM entity WHERE name = $name")
            .bind(("name", name.to_string()))
            .await?
            .take(0)?;
        Ok(result.into_iter().next())
    }

    async fn update_entity(&self, mut entity: Entity) -> Result<Entity> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        entity.updated_at = Datetime::default();
        entity.embedding = Some(self.embed_entity(&entity).await?);

        let mut res = db
            .query("UPDATE entity SET entity_type = $type, observations = $obs, embedding = $embedding, updated_at = $updated WHERE name = $name RETURN AFTER")
            .bind(("name", entity.name.clone()))
            .bind(("type", entity.entity_type.clone()))
            .bind(("obs", entity.observations.clone()))
            .bind(("embedding", entity.embedding.clone()))
            .bind(("updated", entity.updated_at.clone()))
            .await?;
        let updated: Option<Entity> = res.take(0)?;
        updated.context("Failed to update entity")
    }

    async fn delete_entity(&self, name: &str) -> Result<()> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        db.query("DELETE FROM entity WHERE name = $name; DELETE FROM relation WHERE from = $name OR to = $name")
            .bind(("name", name.to_string()))
            .await?;
        Ok(())
    }

    async fn search_entities(&self, query: &str) -> Result<Vec<Entity>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let results: Vec<Entity> = db
            .query("SELECT * FROM entity WHERE name CONTAINS $q OR entity_type CONTAINS $q OR observations CONTAINS $q")
            .bind(("q", query.to_string()))
            .await?
            .take(0)?;
        Ok(results)
    }

    async fn create_relation(&self, mut relation: Relation) -> Result<Relation> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        relation.created_at = Datetime::default();
        let created: Option<Relation> = db
            .create("relation")
            .content(relation)
            .await
            .context("Failed to create relation")?;
        created.ok_or_else(|| anyhow::anyhow!("No relation returned after creation"))
    }

    async fn create_relations(&self, relations: Vec<Relation>) -> Result<Vec<Relation>> {
        let mut results = Vec::with_capacity(relations.len());
        for r in relations {
            results.push(self.create_relation(r).await?);
        }
        Ok(results)
    }

    async fn get_relations(&self, entity_name: &str) -> Result<Vec<Relation>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let results: Vec<Relation> = db
            .query("SELECT * FROM relation WHERE from = $name OR to = $name")
            .bind(("name", entity_name.to_string()))
            .await?
            .take(0)?;
        Ok(results)
    }

    async fn delete_relation(&self, from: &str, to: &str, relation_type: &str) -> Result<()> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        db.query("DELETE FROM relation WHERE from = $from AND to = $to AND relation_type = $rt")
            .bind(("from", from.to_string()))
            .bind(("to", to.to_string()))
            .bind(("rt", relation_type.to_string()))
            .await?;
        Ok(())
    }

    async fn get_graph(&self) -> Result<KnowledgeGraph> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let entities: Vec<Entity> = db.query("SELECT * FROM entity").await?.take(0)?;
        let relations: Vec<Relation> = db.query("SELECT * FROM relation").await?.take(0)?;
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
            .with_context(|| format!("Entity '{}' not found", entity_name))?;
        entity.observations.extend(observations);
        self.update_entity(entity).await
    }

    async fn semantic_search(
        &self,
        query: &str,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<SemanticSearchResult>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let query_emb = self.embed_text(query).await?;
        let all: Vec<Entity> = db
            .query("SELECT * FROM entity WHERE embedding IS NOT NONE")
            .await?
            .take(0)?;

        let mut scored: Vec<SemanticSearchResult> = all
            .into_iter()
            .filter_map(|e| {
                let emb = e.embedding.as_deref()?;
                let sim = Self::cosine_similarity(&query_emb, emb);
                if sim >= threshold {
                    Some(SemanticSearchResult {
                        entity: e,
                        similarity: sim,
                    })
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    // ── Scoped Memory (mem0) ──────────────────────────────────────────────────

    async fn add_memory(&self, mut memory: Memory) -> Result<Memory> {
        // Compute embedding and token count
        let emb = self.embed_text(&memory.content).await?;
        memory.token_count = Some(Self::estimate_tokens(&memory.content));

        // Semantic deduplication at 0.92 threshold
        let candidates = self
            .search_memories(
                &memory.content,
                memory.user_id.as_deref(),
                memory.agent_id.as_deref(),
                memory.session_id.as_deref(),
                None,
                5,
            )
            .await?;

        for candidate in candidates {
            if let Some(c_emb) = &candidate.embedding {
                if Self::cosine_similarity(&emb, c_emb) >= 0.92 {
                    if let Some(id) = &candidate.id {
                        let id_str = Self::record_id_to_string(id);
                        return self.update_memory(&id_str, memory.content).await;
                    }
                }
            }
        }

        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let now = Datetime::default();
        memory.embedding = Some(emb);
        memory.created_at = now.clone();
        memory.updated_at = now;
        memory.version = 1;

        let created: Option<Memory> = db
            .create("memory")
            .content(memory.clone())
            .await
            .context("Failed to create memory")?;

        let stored = created.ok_or_else(|| anyhow::anyhow!("No memory returned after creation"))?;

        // Record history
        if let Some(id) = &stored.id {
            db.query(
                "INSERT INTO memory_history { memory_id: $mid, version: 1, old_content: NONE, new_content: $content, changed_at: $now, change_type: 'created' }"
            )
            .bind(("mid", id.clone()))
            .bind(("content", stored.content.clone()))
            .bind(("now", Datetime::default()))
            .await?;
        }

        Ok(stored)
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let result: Vec<Memory> = db
            .query("SELECT * FROM $id")
            .bind(("id", id.to_string()))
            .await?
            .take(0)?;
        Ok(result.into_iter().next())
    }

    async fn update_memory(&self, id: &str, content: String) -> Result<Memory> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let old = self
            .get_memory(id)
            .await?
            .with_context(|| format!("Memory '{}' not found", id))?;
        let new_emb = self.embed_text(&content).await?;
        let new_version = old.version + 1;
        let token_count = Self::estimate_tokens(&content);
        let now = Datetime::default();

        let mut res = db.query(
            "UPDATE $id SET content = $content, embedding = $emb, token_count = $tc, version = $v, updated_at = $now RETURN AFTER"
        )
        .bind(("id", id.to_string()))
        .bind(("content", content.clone()))
        .bind(("emb", new_emb))
        .bind(("tc", token_count))
        .bind(("v", new_version))
        .bind(("now", now.clone()))
        .await?;

        let updated: Option<Memory> = res.take(0)?;
        let updated = updated.context("Failed to update memory")?;

        // History
        if let Some(mem_id) = &updated.id {
            db.query(
                "INSERT INTO memory_history { memory_id: $mid, version: $v, old_content: $old, new_content: $new, changed_at: $now, change_type: 'updated' }"
            )
            .bind(("mid", mem_id.clone()))
            .bind(("v", new_version))
            .bind(("old", old.content))
            .bind(("new", content))
            .bind(("now", now))
            .await?;
        }

        Ok(updated)
    }

    async fn delete_memory(&self, id: &str) -> Result<()> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        if let Some(mem) = self.get_memory(id).await? {
            if let Some(mem_id) = &mem.id {
                db.query(
                    "INSERT INTO memory_history { memory_id: $mid, version: $v, old_content: $old, new_content: $old, changed_at: $now, change_type: 'deleted' }"
                )
                .bind(("mid", mem_id.clone()))
                .bind(("v", mem.version + 1))
                .bind(("old", mem.content))
                .bind(("now", Datetime::default()))
                .await?;
            }
        }
        db.query("DELETE FROM $id")
            .bind(("id", id.to_string()))
            .await?;
        Ok(())
    }

    async fn delete_all_memories(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<u64> {
        let memories = self.get_all_memories(user_id, agent_id, session_id).await?;
        let count = memories.len() as u64;
        for mem in memories {
            if let Some(id) = &mem.id {
                self.delete_memory(&Self::record_id_to_string(id)).await?;
            }
        }
        Ok(count)
    }

    async fn get_all_memories(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<Memory>> {
        let mut query = String::from("SELECT * FROM memory WHERE ");

        let uid = user_id.map(str::to_string);
        let aid = agent_id.map(str::to_string);
        let sid = session_id.map(str::to_string);

        let mut parts: Vec<String> = vec![];
        if uid.is_some() {
            parts.push("user_id = $user_id".into());
        }
        if aid.is_some() {
            parts.push("agent_id = $agent_id".into());
        }
        if sid.is_some() {
            parts.push("session_id = $session_id".into());
        }
        if parts.is_empty() {
            parts.push("true".into());
        }

        query.push_str(&parts.join(" AND "));

        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let mut q = db.query(query);
        if let Some(v) = uid {
            q = q.bind(("user_id", v));
        }
        if let Some(v) = aid {
            q = q.bind(("agent_id", v));
        }
        if let Some(v) = sid {
            q = q.bind(("session_id", v));
        }

        let results: Vec<Memory> = q.await?.take(0)?;
        Ok(results)
    }

    async fn search_memories(
        &self,
        query: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        _categories: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let query_emb = self.embed_text(query).await?;

        // Get candidates with embedding (filtered by scope)
        let candidates = self.get_all_memories(user_id, agent_id, session_id).await?;

        let mut scored: Vec<(f32, Memory)> = candidates
            .into_iter()
            .filter_map(|m| {
                let emb = m.embedding.as_deref()?;
                let sim = Self::cosine_similarity(&query_emb, emb);
                Some((sim, m))
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(_, m)| m).collect())
    }

    async fn get_memory_history(&self, memory_id: &str) -> Result<Vec<MemoryHistory>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let results: Vec<MemoryHistory> = db
            .query("SELECT * FROM memory_history WHERE memory_id = $mid ORDER BY version ASC")
            .bind(("mid", memory_id.to_string()))
            .await?
            .take(0)?;
        Ok(results)
    }

    // ── TaskStreams ────────────────────────────────────────────────────────────

    async fn create_task_stream(&self, mut stream: TaskStream) -> Result<TaskStream> {
        let now = Datetime::default();
        stream.created_at = now.clone();
        stream.last_active = now;
        let key = Uuid::new_v4().to_string();
        self.create_record("task_stream", &key, stream, "create_task_stream")
            .await
    }

    async fn get_task_stream(&self, name: &str) -> Result<Option<TaskStream>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let result: Vec<TaskStream> = db
            .query("SELECT * FROM task_stream WHERE name = $name")
            .bind(("name", name.to_string()))
            .await?
            .take(0)?;
        Ok(result.into_iter().next())
    }

    async fn add_to_task_stream(&self, stream_name: &str, mut memory: Memory) -> Result<Memory> {
        let stream = self
            .get_task_stream(stream_name)
            .await?
            .with_context(|| format!("TaskStream '{}' not found", stream_name))?;

        if stream.status != TaskStreamStatus::Active {
            anyhow::bail!("TaskStream '{}' is not active", stream_name);
        }

        // Link memory to stream
        if let Some(id) = &stream.id {
            memory.task_stream_id = Some(id.clone());
        }

        let stored = self.add_memory(memory).await?;

        // Update stream token count and last_active
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let added_tokens = stored.token_count.unwrap_or(0) as u64;
        db.query(
            "UPDATE task_stream SET total_tokens += $tokens, last_active = $now WHERE name = $name"
        )
        .bind(("tokens", added_tokens))
        .bind(("now", Datetime::default()))
        .bind(("name", stream_name.to_string()))
        .await?;

        Ok(stored)
    }

    async fn get_context_for_task(
        &self,
        stream_name: &str,
        model_name: &str,
        max_tokens: Option<u64>,
    ) -> Result<ContextWindow> {
        let budget = max_tokens.unwrap_or_else(|| Self::model_context_budget(model_name));

        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let stream_id = {
            let stream = self
                .get_task_stream(stream_name)
                .await?
                .with_context(|| format!("TaskStream '{}' not found", stream_name))?;
            stream.id.with_context(|| "TaskStream has no id")?
        };

        let all_memories: Vec<Memory> = db
            .query("SELECT * FROM memory WHERE task_stream_id = $sid ORDER BY importance DESC, created_at DESC")
            .bind(("sid", stream_id))
            .await?
            .take(0)?;

        let mut included = Vec::new();
        let mut tokens_used: u64 = 0;
        let mut omitted: u64 = 0;

        for mem in all_memories {
            let tc = mem
                .token_count
                .unwrap_or_else(|| Self::estimate_tokens(&mem.content)) as u64;
            if tokens_used + tc <= budget {
                tokens_used += tc;
                included.push(mem);
            } else {
                omitted += 1;
            }
        }

        Ok(ContextWindow {
            memories: included,
            tokens_used,
            memories_omitted: omitted,
            model_name: model_name.to_string(),
            token_budget: budget,
        })
    }

    async fn list_task_streams(
        &self,
        agent_id: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<TaskStream>> {
        let mut parts: Vec<String> = vec![];
        let aid = agent_id.map(str::to_string);
        let uid = user_id.map(str::to_string);

        if aid.is_some() {
            parts.push("agent_id = $agent_id".into());
        }
        if uid.is_some() {
            parts.push("user_id = $user_id".into());
        }
        if parts.is_empty() {
            parts.push("true".into());
        }

        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let query = format!(
            "SELECT * FROM task_stream WHERE {} ORDER BY last_active DESC",
            parts.join(" AND ")
        );
        let mut q = db.query(query);
        if let Some(v) = aid {
            q = q.bind(("agent_id", v));
        }
        if let Some(v) = uid {
            q = q.bind(("user_id", v));
        }

        let results: Vec<TaskStream> = q.await?.take(0)?;
        Ok(results)
    }

    async fn archive_task_stream(&self, name: &str) -> Result<TaskStream> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut res = db
            .query("UPDATE task_stream SET status = 'archived' WHERE name = $name RETURN AFTER")
            .bind(("name", name.to_string()))
            .await?;
        let updated: Option<TaskStream> = res.take(0)?;
        updated.with_context(|| format!("TaskStream '{}' not found", name))
    }

    // ── Hybrid BM25 + HNSW Search ─────────────────────────────────────────────

    async fn hybrid_search_memories(
        &self,
        query: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        limit: usize,
        vector_weight: f32,
        bm25_weight: f32,
    ) -> Result<Vec<Memory>> {
        use std::collections::HashMap;

        // Vector branch (uses HNSW index)
        let vec_results = self
            .search_memories(query, user_id, agent_id, session_id, None, limit * 2)
            .await?;

        // BM25 full-text branch
        let mut scope_parts: Vec<String> = vec!["content @@ $query".into()];
        if user_id.is_some() {
            scope_parts.push("user_id = $uid".into());
        }
        if agent_id.is_some() {
            scope_parts.push("agent_id = $aid".into());
        }
        if session_id.is_some() {
            scope_parts.push("session_id = $sid".into());
        }

        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let bm25_sql = format!(
            "SELECT *, search::score(0) AS bm25_score FROM memory WHERE {} LIMIT {}",
            scope_parts.join(" AND "),
            limit * 2
        );
        let mut q = db.query(&bm25_sql).bind(("query", query.to_string()));
        if let Some(v) = user_id {
            q = q.bind(("uid", v.to_string()));
        }
        if let Some(v) = agent_id {
            q = q.bind(("aid", v.to_string()));
        }
        if let Some(v) = session_id {
            q = q.bind(("sid", v.to_string()));
        }
        let bm25_results: Vec<Memory> = q.await?.take(0).unwrap_or_default();

        // Merge scores: weighted RRF-style normalisation
        let mut scores: HashMap<String, (Memory, f32)> = HashMap::new();
        let n_vec = vec_results.len().max(1) as f32;
        for (i, m) in vec_results.iter().enumerate() {
            let key =
                m.id.as_ref()
                    .map(Self::record_id_to_string)
                    .unwrap_or_default();
            let s = vector_weight * (1.0 - i as f32 / n_vec);
            scores
                .entry(key)
                .and_modify(|(_, sc)| *sc += s)
                .or_insert_with(|| (m.clone(), s));
        }
        let n_bm = bm25_results.len().max(1) as f32;
        for (i, m) in bm25_results.iter().enumerate() {
            let key =
                m.id.as_ref()
                    .map(Self::record_id_to_string)
                    .unwrap_or_default();
            let s = bm25_weight * (1.0 - i as f32 / n_bm);
            scores
                .entry(key)
                .and_modify(|(_, sc)| *sc += s)
                .or_insert_with(|| (m.clone(), s));
        }

        let mut merged: Vec<(Memory, f32)> = scores.into_values().collect();
        merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(merged.into_iter().take(limit).map(|(m, _)| m).collect())
    }

    // ── mem0 Advanced ─────────────────────────────────────────────────────────

    async fn compress_memories(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        older_than_days: u32,
    ) -> Result<Option<Memory>> {
        let mut conditions = vec![format!("created_at < time::now() - {}d", older_than_days)];
        if user_id.is_some() {
            conditions.push("user_id = $uid".into());
        }
        if agent_id.is_some() {
            conditions.push("agent_id = $aid".into());
        }
        if session_id.is_some() {
            conditions.push("session_id = $sid".into());
        }

        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let sql = format!(
            "SELECT * FROM memory WHERE {} ORDER BY created_at ASC",
            conditions.join(" AND ")
        );
        let mut q = db.query(&sql);
        if let Some(v) = user_id {
            q = q.bind(("uid", v.to_string()));
        }
        if let Some(v) = agent_id {
            q = q.bind(("aid", v.to_string()));
        }
        if let Some(v) = session_id {
            q = q.bind(("sid", v.to_string()));
        }
        let old_memories: Vec<Memory> = q.await?.take(0).unwrap_or_default();

        if old_memories.is_empty() {
            return Ok(None);
        }

        let summary_text = old_memories
            .iter()
            .enumerate()
            .map(|(i, m)| format!("{}. {}", i + 1, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        let summary_content = format!(
            "[Compressed {} memories]\n{}",
            old_memories.len(),
            summary_text
        );

        for m in &old_memories {
            if let Some(id) = &m.id {
                let id_str = Self::record_id_to_string(id);
                let _ = self.delete_memory(&id_str).await;
            }
        }

        let summary = Memory::new(
            summary_content,
            user_id.map(str::to_string),
            agent_id.map(str::to_string),
            session_id.map(str::to_string),
            vec!["compressed".to_string()],
        );
        Ok(Some(self.add_memory(summary).await?))
    }

    async fn add_memories_from_conversation(
        &self,
        messages: Vec<serde_json::Value>,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<Memory>> {
        let mut stored = Vec::new();
        for msg in &messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if content.is_empty() {
                continue;
            }
            let mut memory = Memory::new(
                content,
                user_id.map(str::to_string),
                agent_id.map(str::to_string),
                session_id.map(str::to_string),
                vec!["conversation".to_string(), role.to_string()],
            );
            memory.memory_type = crate::memory::MemoryType::Episodic;
            match self.add_memory(memory).await {
                Ok(m) => stored.push(m),
                Err(e) => tracing::warn!("Skipping conversation message: {}", e),
            }
        }
        Ok(stored)
    }

    async fn expire_stale_memories(&self) -> Result<u64> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let expired: Vec<Memory> = db
            .query(
                "SELECT * FROM memory WHERE valid_until IS NOT NONE AND valid_until < time::now()",
            )
            .await?
            .take(0)
            .unwrap_or_default();
        let count = expired.len() as u64;
        for m in &expired {
            if let Some(id) = &m.id {
                let _ = self.delete_memory(&Self::record_id_to_string(id)).await;
            }
        }
        tracing::info!("Expired {} stale memories", count);
        Ok(count)
    }

    // ── Mindmaps ─────────────────────────────────────────────────────────────

    async fn create_mindmap(&self, mut mindmap: MindMap) -> Result<MindMap> {
        mindmap.created_at = Datetime::default();
        mindmap.updated_at = mindmap.created_at.clone();
        let key = Uuid::new_v4().to_string();
        self.create_record("mindmap", &key, mindmap, "create_mindmap")
            .await
    }

    async fn get_mindmap(&self, name: &str, user_id: Option<&str>) -> Result<Option<MindMap>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut sql = "SELECT * FROM mindmap WHERE name = $name".to_string();
        if user_id.is_some() {
            sql.push_str(" AND user_id = $uid");
        }
        let mut q = db.query(&sql).bind(("name", name.to_string()));
        if let Some(v) = user_id {
            q = q.bind(("uid", v.to_string()));
        }
        let mut results: Vec<MindMap> = q.await?.take(0)?;
        Ok(results.pop())
    }

    async fn add_mindmap_node(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        node: crate::mindmap::MindMapNode,
    ) -> Result<MindMap> {
        let mut mm = self
            .get_mindmap(mindmap_name, user_id)
            .await?
            .with_context(|| format!("Mindmap '{}' not found", mindmap_name))?;
        if mm.nodes.iter().any(|n| n.id == node.id) {
            anyhow::bail!("Node '{}' already exists", node.id);
        }
        mm.nodes.push(node);
        mm.updated_at = Datetime::default();
        let record_key = mm
            .id
            .as_ref()
            .map(Self::record_id_to_string)
            .context("Mindmap missing id")?;
        self.replace_record(&record_key, mm, "add_mindmap_node")
            .await
    }

    async fn add_mindmap_edge(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        edge: crate::mindmap::MindMapEdge,
    ) -> Result<MindMap> {
        let mut mm = self
            .get_mindmap(mindmap_name, user_id)
            .await?
            .with_context(|| format!("Mindmap '{}' not found", mindmap_name))?;
        mm.edges.push(edge);
        mm.updated_at = Datetime::default();
        let record_key = mm
            .id
            .as_ref()
            .map(Self::record_id_to_string)
            .context("Mindmap missing id")?;
        self.replace_record(&record_key, mm, "add_mindmap_edge")
            .await
    }

    async fn delete_mindmap_node(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        node_id: &str,
    ) -> Result<MindMap> {
        let mut mm = self
            .get_mindmap(mindmap_name, user_id)
            .await?
            .with_context(|| format!("Mindmap '{}' not found", mindmap_name))?;
        mm.nodes.retain(|n| n.id != node_id);
        mm.edges
            .retain(|e| e.from_id != node_id && e.to_id != node_id);
        mm.updated_at = Datetime::default();
        let record_key = mm
            .id
            .as_ref()
            .map(Self::record_id_to_string)
            .context("Mindmap missing id")?;
        self.replace_record(&record_key, mm, "delete_mindmap_node")
            .await
    }

    async fn list_mindmaps(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Vec<MindMap>> {
        let mut parts: Vec<String> = vec![];
        if user_id.is_some() {
            parts.push("user_id = $uid".into());
        }
        if agent_id.is_some() {
            parts.push("agent_id = $aid".into());
        }
        let sql = if parts.is_empty() {
            "SELECT * FROM mindmap ORDER BY updated_at DESC".to_string()
        } else {
            format!(
                "SELECT * FROM mindmap WHERE {} ORDER BY updated_at DESC",
                parts.join(" AND ")
            )
        };
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut q = db.query(&sql);
        if let Some(v) = user_id {
            q = q.bind(("uid", v.to_string()));
        }
        if let Some(v) = agent_id {
            q = q.bind(("aid", v.to_string()));
        }
        Ok(q.await?.take(0).unwrap_or_default())
    }

    async fn delete_mindmap(&self, name: &str, user_id: Option<&str>) -> Result<()> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        if let Some(mm) = self.get_mindmap(name, user_id).await? {
            if let Some(id) = &mm.id {
                let s = Self::record_id_to_string(id);
                let key = s.split(':').nth(1).unwrap_or(&s).to_string();
                let _: Option<MindMap> = db.delete(("mindmap", key)).await?;
            }
        }
        Ok(())
    }

    // ── Phase 3: Advanced Context + Graph-RAG ────────────────────────────────

    async fn auto_summarize_task_stream(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        model_id: &str,
    ) -> Result<Option<crate::memory::Memory>> {
        use crate::model_profiles::profile_for;
        use crate::task_stream::TaskStreamStatus;

        // Load the stream
        let Some(stream) = self.get_task_stream(stream_name).await? else {
            return Ok(None);
        };
        if stream.status != TaskStreamStatus::Active {
            return Ok(None);
        }

        let profile = profile_for(model_id);
        if stream.total_tokens < profile.summarization_threshold() {
            return Ok(None); // nothing to do
        }

        // Fetch stream memories ordered oldest-first
        let mut scope_parts: Vec<String> = vec!["task_stream_id != NONE".into()];
        if let Some(u) = user_id {
            scope_parts.push(format!("user_id = '{}'", u));
        }
        if let Some(a) = agent_id {
            scope_parts.push(format!("agent_id = '{}'", a));
        }
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let sql = format!(
            "SELECT * FROM memory WHERE {} ORDER BY created_at ASC LIMIT 200",
            scope_parts.join(" AND ")
        );
        let memories: Vec<crate::memory::Memory> =
            db.query(&sql).await?.take(0).unwrap_or_default();

        if memories.len() < 4 {
            return Ok(None); // not enough to compress
        }

        // Compress the oldest half
        let half = memories.len() / 2;
        let to_compress = memories.into_iter().take(half).collect::<Vec<_>>();

        // Build summary content
        let summary_content = format!(
            "[Auto-summary of {} memories from task stream '{}'] {}",
            half,
            stream_name,
            to_compress
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        );

        // Delete the originals
        for m in &to_compress {
            if let Some(id) = &m.id {
                let s = Self::record_id_to_string(id);
                let key = s.split(':').nth(1).unwrap_or(&s).to_string();
                let _: Option<crate::memory::Memory> = db.delete(("memory", key)).await?;
            }
        }

        // Store the summary
        let summary = crate::memory::Memory::new(
            summary_content,
            user_id.map(str::to_string),
            agent_id.map(str::to_string),
            None,
            vec!["auto_summary".to_string()],
        );
        let stored: Option<crate::memory::Memory> =
            db.create("memory").content(summary).await?;
        let result = stored.context("Failed to store auto-summary memory")?;

        // Update stream summary_count
        let _: Option<serde_json::Value> = db
            .query("UPDATE type::table($t) SET summary_count += 1, last_active = time::now() WHERE name = $n")
            .bind(("t", "task_stream"))
            .bind(("n", stream_name.to_string()))
            .await?
            .take(0)?;

        tracing::info!(
            "Auto-summarized {} memories in task stream '{}' → 1 summary",
            half,
            stream_name
        );
        Ok(Some(result))
    }

    async fn try_update_persona_mindmap(
        &self,
        user_id: &str,
        memory: &crate::memory::Memory,
    ) -> Result<()> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        // Look for a persona mindmap belonging to this user
        let maps: Vec<MindMap> = db
            .query("SELECT * FROM mindmap WHERE user_id = $uid AND map_type = 'radial' LIMIT 1")
            .bind(("uid", user_id.to_string()))
            .await?
            .take(0)
            .unwrap_or_default();

        let Some(mm) = maps.into_iter().next() else {
            return Ok(()); // no persona mindmap exists yet
        };

        // Find best parent branch by matching category
        let category = memory
            .categories
            .first()
            .cloned()
            .unwrap_or_else(|| "general".to_string());
        let parent_id = mm
            .nodes
            .iter()
            .find(|n| n.label.to_lowercase().contains(&category.to_lowercase()))
            .map(|n| n.id.clone())
            .or_else(|| mm.nodes.first().map(|n| n.id.clone()))
            .unwrap_or_else(|| "root".to_string());

        let snippet: String = memory.content.chars().take(80).collect();
        let node_id = format!("mem_{}", chrono_node_id());
        let node = crate::mindmap::MindMapNode {
            id: node_id,
            label: snippet,
            parent_id: Some(parent_id),
            node_type: Some("memory".to_string()),
            color: None,
            metadata: None,
        };

        self.add_mindmap_node(&mm.name, Some(user_id), node).await?;
        Ok(())
    }

    async fn find_path(&self, from: &str, to: &str, max_depth: u8) -> Result<Vec<Vec<String>>> {
        // BFS over the relation table in Rust using relation lookups
        // (A future enhancement can push this to SurrealQL graph traversal syntax)
        let depth = (max_depth as usize).min(6);
        let mut found: Vec<Vec<String>> = vec![];
        let mut queue: Vec<Vec<String>> = vec![vec![from.to_string()]];

        for _hop in 0..depth {
            let mut next_queue: Vec<Vec<String>> = vec![];
            for path in &queue {
                let current = path.last().unwrap();
                if current == to {
                    found.push(path.clone());
                    continue;
                }
                if found.len() >= 5 {
                    break;
                }
                // Expand forward relations
                let db = {
                    let state = self.connection.read().unwrap();
                    match &*state {
                        ConnectionState::Connected(db) => db.clone(),
                        ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                        ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
                    }
                };
                let neighbors: Vec<crate::entity::Relation> = db
                    .query("SELECT * FROM relation WHERE from = $entity")
                    .bind(("entity", current.clone()))
                    .await?
                    .take(0)
                    .unwrap_or_default();
                for rel in neighbors {
                    if !path.contains(&rel.to) {
                        let mut new_path = path.clone();
                        new_path.push(rel.to);
                        next_queue.push(new_path);
                    }
                }
            }
            queue = next_queue;
            if found.len() >= 5 || queue.is_empty() {
                break;
            }
        }
        // Also capture any path that ends at `to`
        queue.retain(|p| p.last().map(|e| e == to).unwrap_or(false));
        found.extend(queue.into_iter().take(5 - found.len()));
        Ok(found)
    }

    async fn expand_neighbors(
        &self,
        entity_name: &str,
        depth: u8,
        limit: usize,
    ) -> Result<crate::entity::KnowledgeGraph> {
        let depth = (depth as usize).min(5);
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut entities: Vec<crate::entity::Entity> = vec![];
        let mut relations: Vec<crate::entity::Relation> = vec![];
        let mut frontier: Vec<String> = vec![entity_name.to_string()];
        visited.insert(entity_name.to_string());

        for _hop in 0..depth {
            if frontier.is_empty() || entities.len() >= limit {
                break;
            }
            let mut next_frontier: Vec<String> = vec![];
            for name in frontier.drain(..) {
                if let Some(e) = self.get_entity(&name).await? {
                    entities.push(e);
                }
                let db = {
                    let state = self.connection.read().unwrap();
                    match &*state {
                        ConnectionState::Connected(db) => db.clone(),
                        ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                        ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
                    }
                };
                let rels: Vec<crate::entity::Relation> = db
                    .query("SELECT * FROM relation WHERE from = $n OR to = $n")
                    .bind(("n", name.clone()))
                    .await?
                    .take(0)
                    .unwrap_or_default();
                for r in rels {
                    let neighbor = if r.from == name {
                        r.to.clone()
                    } else {
                        r.from.clone()
                    };
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor.clone());
                        next_frontier.push(neighbor);
                    }
                    if !relations.iter().any(|x| {
                        x.from == r.from && x.to == r.to && x.relation_type == r.relation_type
                    }) {
                        relations.push(r);
                    }
                }
            }
            frontier = next_frontier;
        }
        Ok(crate::entity::KnowledgeGraph {
            entities,
            relations,
        })
    }

    async fn get_related(
        &self,
        entity_name: &str,
        relation_type: Option<&str>,
        direction: &str,
        limit: usize,
    ) -> Result<Vec<crate::entity::Entity>> {
        // Build query depending on direction
        let mut conditions: Vec<String> = vec![];
        match direction {
            "in" => conditions.push(format!("to = '{}'", entity_name)),
            "out" => conditions.push(format!("from = '{}'", entity_name)),
            _ => conditions.push(format!("(from = '{0}' OR to = '{0}')", entity_name)),
        }
        if let Some(rt) = relation_type {
            conditions.push(format!("relation_type = '{}'", rt));
        }
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let sql = format!(
            "SELECT * FROM relation WHERE {} LIMIT {}",
            conditions.join(" AND "),
            limit
        );
        let rels: Vec<crate::entity::Relation> =
            db.query(&sql).await?.take(0).unwrap_or_default();

        let mut entities: Vec<crate::entity::Entity> = vec![];
        for rel in rels {
            let neighbor = if direction == "in" {
                &rel.from
            } else {
                &rel.to
            };
            if let Some(e) = self.get_entity(neighbor).await? {
                entities.push(e);
            }
        }
        Ok(entities)
    }

    // ── Phase 4: Temporal Entity History ─────────────────────────────────────

    async fn get_entity_history(&self, name: &str) -> Result<Vec<crate::memory::MemoryHistory>> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let rows: Vec<crate::memory::MemoryHistory> = db
            .query("SELECT * FROM memory_history WHERE memory_id = $n ORDER BY changed_at DESC")
            .bind(("n", name.to_string()))
            .await?
            .take(0)
            .unwrap_or_default();
        Ok(rows)
    }

    async fn get_graph_at_time(
        &self,
        before_rfc3339: &str,
    ) -> Result<crate::entity::KnowledgeGraph> {
        let db = {
            let state = self.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let entities: Vec<crate::entity::Entity> = db
            .query("SELECT * FROM entity WHERE created_at <= type::datetime($t)")
            .bind(("t", before_rfc3339.to_string()))
            .await?
            .take(0)
            .unwrap_or_default();
        let relations: Vec<crate::entity::Relation> = db
            .query("SELECT * FROM relation WHERE created_at <= type::datetime($t)")
            .bind(("t", before_rfc3339.to_string()))
            .await?
            .take(0)
            .unwrap_or_default();
        Ok(crate::entity::KnowledgeGraph {
            entities,
            relations,
        })
    }
}

/// Simple monotonic node ID for mindmap auto-update leaves.
fn chrono_node_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

// ── Test/Utility helpers ──────────────────────────────────────────────────────

impl SurrealStorage {
    /// Creates a `SurrealStorage` backed by an **embedded RocksDB in a unique temp directory**.
    /// Each call uses a nanosecond-timestamped path so parallel tests don't collide.
    /// `mem://` is unavailable in surrealdb 3.0.0 (`kv-mem` requires surrealmx ≥ 0.17).
    pub async fn new_mem(
        embedding_service: Arc<dyn crate::embeddings::EmbeddingService>,
    ) -> Result<Self> {
        use crate::storage::migrations::run_migrations;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| format!("{}{}", d.as_secs(), d.subsec_nanos()))
            .unwrap_or_else(|_| "0".to_string());
        let dir = std::env::temp_dir().join(format!("surreal-test-{}", ts));
        std::fs::create_dir_all(&dir)?;
        let path = dir.display().to_string();

        let config = SurrealConfig {
            mode: SurrealMode::Embedded,
            embedded_path: Some(path),
            namespace: "test".to_string(),
            database: "test".to_string(),
            ..Default::default()
        };

        let connection_info = ConnectionInfo {
            config: config.clone(),
            retry_config: config.retry.clone(),
        };

        let db = Self::connect_with_config(&config).await?;
        run_migrations(&db).await?;

        let connection = Arc::new(RwLock::new(ConnectionState::Connected(db)));

        Ok(Self {
            connection,
            connection_info,
            embedding_service,
        })
    }
}

// ── Retry configuration tests ─────────────────────────────────────────────────

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_connect_retries, 10);
        assert_eq!(config.max_operation_retries, 3);
        assert_eq!(config.base_retry_delay_ms, 100);
        assert_eq!(config.max_retry_delay_ms, 5000);
        assert_eq!(config.jitter_factor, 0.25);
    }

    #[test]
    fn test_exponential_backoff_calculation() {
        let config = RetryConfig::default();
        let delay1 = config.calculate_delay(0);
        let delay2 = config.calculate_delay(1);
        let delay3 = config.calculate_delay(2);

        // Base delays (before jitter): 100ms, 200ms, 400ms
        assert!(delay1.as_millis() >= 75 && delay1.as_millis() <= 125); // 100 ± 25%
        assert!(delay2.as_millis() >= 150 && delay2.as_millis() <= 250); // 200 ± 25%
        assert!(delay3.as_millis() >= 300 && delay3.as_millis() <= 500); // 400 ± 25%
    }

    #[test]
    fn test_backoff_respects_max_delay() {
        let config = RetryConfig {
            max_retry_delay_ms: 500,
            ..Default::default()
        };
        let delay = config.calculate_delay(10); // Would be > 500ms without cap
        assert!(delay.as_millis() <= 625); // 500 + 25% jitter
    }

    #[tokio::test]
    async fn test_connection_state_lifecycle() {
        // Mock connection (will fail to actually connect, but tests the type)
        let state = ConnectionState::Reconnecting;
        assert!(matches!(state, ConnectionState::Reconnecting));

        let state = ConnectionState::Failed("test error".to_string());
        if let ConnectionState::Failed(msg) = state {
            assert_eq!(msg, "test error");
        } else {
            panic!("Expected Failed state");
        }
    }
}
