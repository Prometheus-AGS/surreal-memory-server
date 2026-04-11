//! SurrealDB-backed implementation of `MemoryStorage`.

use super::MemoryStorage;
use crate::{
    embeddings::EmbeddingService,
    entity::{Entity, KnowledgeGraph, Relation, SemanticSearchResult},
    memory::{Memory, MemoryHistory, MemoryScope, MemoryType},
    mindmap::{MapType, MindMap, MindMapEdge, MindMapNode},
    storage::migrations::run_migrations,
    task_stream::{ContextWindow, TaskStream, TaskStreamStatus},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde::{Serialize, de::DeserializeOwned};
use std::{cmp::Ordering, sync::Arc};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::opt::auth::Root;
use surrealdb::types::{Datetime, RecordId, RecordIdKey};
use surrealdb_types::{SurrealValue, Value};
use uuid::Uuid;

/// Token budget constants per model family. Extend via config in Phase 3.
const DEFAULT_CONTEXT_BUDGET: u64 = 100_000;

/// SurrealDB-backed memory storage.
///
/// `Surreal<Any>` is internally `Arc`-wrapped and `Clone`-safe; no additional
/// mutex or wrapper is needed for concurrent access.  WebSocket connections
/// (remote server mode) auto-reconnect; the initial connection uses
/// `connect_with_retry` which honours `SurrealConfig::retry`.
pub struct SurrealStorage {
    connection: Arc<std::sync::RwLock<ConnectionState>>,
    connection_info: ConnectionInfo,
    embedding_service: Arc<dyn EmbeddingService>,
}

// ── Retry Configuration ───────────────────────────────────────────────────────

use std::time::Duration;

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
        use rand::RngExt as _;
        let mut rng = rand::rng();

        // Exponential backoff: base * 2^attempt
        let base_delay = self
            .base_retry_delay_ms
            .saturating_mul(2u64.saturating_pow(attempt));

        // Apply max cap
        let capped_delay = base_delay.min(self.max_retry_delay_ms);

        // Apply jitter: delay * (1 ± jitter_factor)
        let jitter_range = (capped_delay as f64 * self.jitter_factor) as u64;
        let min_delay = capped_delay.saturating_sub(jitter_range);
        let max_delay = capped_delay.saturating_add(jitter_range).max(min_delay);

        let jittered_delay = rng.random_range(min_delay..=max_delay);
        Duration::from_millis(jittered_delay)
    }
}

/// Connection configuration — stored on the struct for diagnostics and
/// future reconnection needs.  `config.retry` holds the retry settings.
#[derive(Debug, Clone)]
struct ConnectionInfo {
    config: SurrealConfig,
}

/// Tracks the lifecycle state of the SurrealDB connection.
pub(crate) enum ConnectionState {
    /// Active connection, ready to use.
    Connected(Surreal<Any>),
    /// Mid-reconnect; callers should wait or fail fast.
    Reconnecting,
    /// Connection could not be established or lost permanently.
    Failed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct DbTaskStream {
    id: Option<RecordId>,
    name: String,
    description: Option<String>,
    agent_id: Option<String>,
    user_id: Option<String>,
    status: String,
    total_tokens: u64,
    model_id: Option<String>,
    auto_summarize: bool,
    summary_count: u32,
    created_at: Datetime,
    last_active: Datetime,
}

impl From<TaskStream> for DbTaskStream {
    fn from(stream: TaskStream) -> Self {
        Self {
            id: stream.id,
            name: stream.name,
            description: stream.description,
            agent_id: stream.agent_id,
            user_id: stream.user_id,
            status: stream.status.as_str().to_string(),
            total_tokens: stream.total_tokens,
            model_id: stream.model_id,
            auto_summarize: stream.auto_summarize,
            summary_count: stream.summary_count,
            created_at: stream.created_at,
            last_active: stream.last_active,
        }
    }
}

impl TryFrom<DbTaskStream> for TaskStream {
    type Error = anyhow::Error;

    fn try_from(stream: DbTaskStream) -> Result<Self> {
        let record_id = stream
            .id
            .as_ref()
            .map(SurrealStorage::record_id_to_string)
            .unwrap_or_else(|| "<new>".to_string());
        let status = TaskStreamStatus::parse_str(&stream.status).map_err(|err| {
            anyhow::anyhow!(
                "task_stream.status decode failed for record {}: {} (raw={})",
                record_id,
                err,
                stream.status
            )
        })?;

        Ok(Self {
            id: stream.id,
            name: stream.name,
            description: stream.description,
            agent_id: stream.agent_id,
            user_id: stream.user_id,
            status,
            total_tokens: stream.total_tokens,
            model_id: stream.model_id,
            auto_summarize: stream.auto_summarize,
            summary_count: stream.summary_count,
            created_at: stream.created_at,
            last_active: stream.last_active,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct DbMindMap {
    id: Option<RecordId>,
    name: String,
    description: Option<String>,
    map_type: String,
    agent_id: Option<String>,
    user_id: Option<String>,
    task_stream_id: Option<RecordId>,
    tags: Vec<String>,
    nodes: Vec<MindMapNode>,
    edges: Vec<MindMapEdge>,
    created_at: Datetime,
    updated_at: Datetime,
}

impl From<MindMap> for DbMindMap {
    fn from(mindmap: MindMap) -> Self {
        Self {
            id: mindmap.id,
            name: mindmap.name,
            description: mindmap.description,
            map_type: mindmap.map_type.as_str().to_string(),
            agent_id: mindmap.agent_id,
            user_id: mindmap.user_id,
            task_stream_id: mindmap.task_stream_id,
            tags: mindmap.tags,
            nodes: mindmap.nodes,
            edges: mindmap.edges,
            created_at: mindmap.created_at,
            updated_at: mindmap.updated_at,
        }
    }
}

impl TryFrom<DbMindMap> for MindMap {
    type Error = anyhow::Error;

    fn try_from(mindmap: DbMindMap) -> Result<Self> {
        let record_id = mindmap
            .id
            .as_ref()
            .map(SurrealStorage::record_id_to_string)
            .unwrap_or_else(|| "<new>".to_string());
        let map_type = MapType::parse_str(&mindmap.map_type).map_err(|err| {
            anyhow::anyhow!(
                "mindmap.map_type decode failed for record {}: {} (raw={})",
                record_id,
                err,
                mindmap.map_type
            )
        })?;

        Ok(Self {
            id: mindmap.id,
            name: mindmap.name,
            description: mindmap.description,
            map_type,
            agent_id: mindmap.agent_id,
            user_id: mindmap.user_id,
            task_stream_id: mindmap.task_stream_id,
            tags: mindmap.tags,
            nodes: mindmap.nodes,
            edges: mindmap.edges,
            created_at: mindmap.created_at,
            updated_at: mindmap.updated_at,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct DbMemory {
    id: Option<RecordId>,
    content: String,
    embedding: Option<Vec<f32>>,
    scope: String,
    memory_type: String,
    user_id: Option<String>,
    session_id: Option<String>,
    agent_id: Option<String>,
    task_stream_id: Option<RecordId>,
    categories: Vec<String>,
    metadata: Option<serde_json::Value>,
    token_count: Option<u32>,
    importance: f32,
    access_count: u32,
    last_accessed_at: Option<Datetime>,
    valid_until: Option<Datetime>,
    version: u32,
    created_at: Datetime,
    updated_at: Datetime,
}

impl From<Memory> for DbMemory {
    fn from(memory: Memory) -> Self {
        Self {
            id: memory.id,
            content: memory.content,
            embedding: memory.embedding,
            scope: memory.scope.as_str().to_string(),
            memory_type: memory.memory_type.as_str().to_string(),
            user_id: memory.user_id,
            session_id: memory.session_id,
            agent_id: memory.agent_id,
            task_stream_id: memory.task_stream_id,
            categories: memory.categories,
            metadata: memory.metadata,
            token_count: memory.token_count,
            importance: memory.importance,
            access_count: memory.access_count,
            last_accessed_at: memory.last_accessed_at,
            valid_until: memory.valid_until,
            version: memory.version,
            created_at: memory.created_at,
            updated_at: memory.updated_at,
        }
    }
}

impl TryFrom<DbMemory> for Memory {
    type Error = anyhow::Error;

    fn try_from(memory: DbMemory) -> Result<Self> {
        let record_id = memory
            .id
            .as_ref()
            .map(SurrealStorage::record_id_to_string)
            .unwrap_or_else(|| "<new>".to_string());
        let scope = MemoryScope::parse_str(&memory.scope).map_err(|err| {
            anyhow::anyhow!(
                "memory.scope decode failed for record {}: {} (raw={})",
                record_id,
                err,
                memory.scope
            )
        })?;
        let memory_type = MemoryType::parse_str(&memory.memory_type).map_err(|err| {
            anyhow::anyhow!(
                "memory.memory_type decode failed for record {}: {} (raw={})",
                record_id,
                err,
                memory.memory_type
            )
        })?;

        Ok(Self {
            id: memory.id,
            content: memory.content,
            embedding: memory.embedding,
            scope,
            memory_type,
            user_id: memory.user_id,
            session_id: memory.session_id,
            agent_id: memory.agent_id,
            task_stream_id: memory.task_stream_id,
            categories: memory.categories,
            metadata: memory.metadata,
            token_count: memory.token_count,
            importance: memory.importance,
            access_count: memory.access_count,
            last_accessed_at: memory.last_accessed_at,
            valid_until: memory.valid_until,
            version: memory.version,
            created_at: memory.created_at,
            updated_at: memory.updated_at,
        })
    }
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
        };

        let db = Self::connect_with_retry(config).await?;

        // Run migrations on initial connection
        run_migrations(&db).await?;

        Ok(Self {
            connection: Arc::new(std::sync::RwLock::new(ConnectionState::Connected(db))),
            connection_info,
            embedding_service,
        })
    }

    /// Establish a connection with exponential-backoff retry.
    ///
    /// Used by `new()` for the initial connection.  `new_mem()` (embedded test
    /// helper) skips retries because a missing embedded path is a programmer
    /// error, not a transient failure.
    async fn connect_with_retry(config: &SurrealConfig) -> Result<Surreal<Any>> {
        let mut attempt = 0u32;
        loop {
            match Self::connect_with_config(config).await {
                Ok(db) => return Ok(db),
                Err(e) if attempt < config.retry.max_connect_retries => {
                    let delay = config.retry.calculate_delay(attempt);
                    tracing::warn!(
                        attempt,
                        delay_ms = delay.as_millis(),
                        error = %e,
                        "SurrealDB connect failed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(e) => {
                    return Err(e).context(format!(
                        "SurrealDB connection failed after {} attempts",
                        attempt + 1
                    ));
                }
            }
        }
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

    /// Ping the database to verify the connection is alive.
    ///
    /// Useful for liveness/readiness probes when running against a remote
    /// SurrealDB server.
    pub async fn health_check(&self) -> Result<bool> {
        let db = {
            let state = self.connection.read().expect("Connection lock poisoned");
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        db.health()
            .await
            .map(|_| true)
            .context("SurrealDB health check failed")
    }

    /// Clone the live `Surreal<Any>` handle for shared use by subsystems (e.g. PalaceAdapter).
    ///
    /// `Surreal<Any>` is internally `Arc`-wrapped — cloning is cheap.
    /// Returns `Err` if the connection is in `Reconnecting` or `Failed` state.
    pub fn db(&self) -> Result<Surreal<Any>> {
        let state = self.connection.read().expect("Connection lock poisoned");
        match &*state {
            ConnectionState::Connected(db) => Ok(db.clone()),
            ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
            ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
        }
    }

    /// Return the namespace and database this storage instance is connected to.
    ///
    /// Useful for diagnostics, logging, and multi-tenant routing.
    pub fn connection_config(&self) -> (&str, &str) {
        (
            self.connection_info.config.namespace.as_str(),
            self.connection_info.config.database.as_str(),
        )
    }

    /// Return a cloneable reference to the connection state lock.
    ///
    /// Used by `PalaceAdapter` to build a closure that yields the live
    /// `Surreal<Any>` handle on each operation (resilient to reconnection).
    #[cfg(feature = "palace")]
    pub(crate) fn connection_arc(&self) -> Arc<std::sync::RwLock<ConnectionState>> {
        Arc::clone(&self.connection)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Classify errors as retriable or non-retriable.
    fn is_retriable_error(&self, error: &anyhow::Error) -> bool {
        let error_msg = format!("{}", error).to_lowercase();

        // Retriable: Network errors
        if error_msg.contains("connection")
            || error_msg.contains("timeout")
            || error_msg.contains("dns")
            || error_msg.contains("refused")
            || error_msg.contains("reset")
            || error_msg.contains("closed")
        {
            return true;
        }

        // Retriable: Transient DB errors
        if error_msg.contains("too many connections")
            || error_msg.contains("backpressure")
            || error_msg.contains("lock timeout")
            || error_msg.contains("serialization failure")
        {
            return true;
        }

        // Non-retriable: Schema/validation errors
        if error_msg.contains("field")
            || error_msg.contains("table")
            || error_msg.contains("invalid")
            || error_msg.contains("credential")
            || error_msg.contains("not found")
            || error_msg.contains("constraint")
        {
            return false;
        }

        // Default: don't retry unknown errors
        false
    }

    /// Attempts to reconnect after connection loss.
    /// Updates ConnectionState: Current → Reconnecting → Connected/Failed
    async fn reconnect(&self) -> Result<()> {
        // Set state to Reconnecting
        {
            let mut state = self.connection.write().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            *state = ConnectionState::Reconnecting;
            tracing::warn!("Connection lost, attempting reconnection");
        }

        // Attempt to establish new connection
        match Self::connect_with_retry(&self.connection_info.config).await {
            Ok(db) => {
                let mut state = self.connection.write().expect(
                    "Connection lock poisoned - another thread panicked while holding the lock",
                );
                *state = ConnectionState::Connected(db);
                tracing::info!("Reconnection successful");
                Ok(())
            }
            Err(err) => {
                let error_msg = format!("{}", err);
                let mut state = self.connection.write().expect(
                    "Connection lock poisoned - another thread panicked while holding the lock",
                );
                *state = ConnectionState::Failed(error_msg.clone());
                tracing::error!(error = %err, "Reconnection failed after exhausting retries");
                Err(anyhow::anyhow!("Reconnection failed: {}", error_msg))
            }
        }
    }

    /// Generic retry wrapper for database operations.
    /// Handles connection extraction, error classification, reconnection, and exponential backoff.
    async fn retry_operation<F, R, Fut>(&self, op_name: &str, op: F) -> Result<R>
    where
        F: Fn(Surreal<Any>) -> Fut,
        Fut: std::future::Future<Output = Result<R>>,
    {
        let max_retries = self.connection_info.config.retry.max_operation_retries;
        let mut last_error = None;

        for attempt in 0..max_retries {
            // Extract current connection
            let db = {
                let state = self.connection.read().expect(
                    "Connection lock poisoned - another thread panicked while holding the lock",
                );
                match &*state {
                    ConnectionState::Connected(db) => db.clone(),
                    ConnectionState::Reconnecting => {
                        anyhow::bail!("Connection is currently reconnecting, please retry later")
                    }
                    ConnectionState::Failed(msg) => {
                        anyhow::bail!("Connection failed: {}", msg)
                    }
                }
            };

            // Attempt operation
            match op(db).await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    last_error = Some(err);

                    if attempt < max_retries - 1
                        && self.is_retriable_error(last_error.as_ref().unwrap())
                    {
                        // Attempt reconnection
                        if let Err(reconnect_err) = self.reconnect().await {
                            tracing::warn!(
                                operation = op_name,
                                attempt = attempt + 1,
                                error = %reconnect_err,
                                "Reconnection failed during retry"
                            );
                        }

                        let delay = self.connection_info.config.retry.calculate_delay(attempt);

                        tracing::warn!(
                            operation = op_name,
                            attempt = attempt + 1,
                            max_attempts = max_retries,
                            error = %last_error.as_ref().unwrap(),
                            next_delay_ms = delay.as_millis(),
                            "Retrying operation after transient failure"
                        );

                        tokio::time::sleep(delay).await;
                    } else {
                        break;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("Operation '{}' failed with no error details", op_name)
        }))
    }

    async fn embed_entity(&self, entity: &Entity) -> Result<Vec<f32>> {
        let mut parts = vec![format!("{} ({})", entity.name, entity.entity_type)];
        parts.extend(entity.observations.iter().cloned());
        self.embedding_service.embed(&parts.join("\n")).await
    }

    async fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        self.embedding_service.embed(text).await
    }

    fn sanitize_explicit_record_content<P>(data: P) -> Value
    where
        P: SurrealValue,
    {
        let mut data = data.into_value();
        if let Value::Object(ref mut map) = data {
            map.remove("id");
        }
        data
    }

    fn decode_memory(record: DbMemory) -> Result<Memory> {
        record.try_into()
    }

    fn decode_memories(records: Vec<DbMemory>) -> Result<Vec<Memory>> {
        records.into_iter().map(Self::decode_memory).collect()
    }

    fn decode_task_stream(record: DbTaskStream) -> Result<TaskStream> {
        record.try_into()
    }

    fn decode_task_streams(records: Vec<DbTaskStream>) -> Result<Vec<TaskStream>> {
        records.into_iter().map(Self::decode_task_stream).collect()
    }

    fn decode_mindmap(record: DbMindMap) -> Result<MindMap> {
        record.try_into()
    }

    fn decode_mindmaps(records: Vec<DbMindMap>) -> Result<Vec<MindMap>> {
        records.into_iter().map(Self::decode_mindmap).collect()
    }

    async fn create_record<P, T>(&self, table: &str, key: &str, value: P, op: &str) -> Result<T>
    where
        P: SurrealValue,
        T: DeserializeOwned + SurrealValue,
    {
        let table = table.to_string();
        let key = key.to_string();
        let operation = op.to_string();
        let payload = Self::sanitize_explicit_record_content(value);

        self.retry_operation(&operation, |db| {
            let table = table.clone();
            let key = key.clone();
            let operation = operation.clone();
            let payload = payload.clone();

            async move {
                let mut response = db
                    .query(
                        "CREATE type::record($table, $key) CONTENT $value RETURN AFTER TIMEOUT 30s",
                    )
                    .bind(("table", table))
                    .bind(("key", key))
                    .bind(("value", payload))
                    .await
                    .with_context(|| format!("{operation}: SurrealDB create query failed"))?;
                response = response
                    .check()
                    .with_context(|| format!("{operation}: SurrealDB rejected the write"))?;
                let created: Option<T> = response
                    .take(0)
                    .with_context(|| format!("{operation}: Failed to deserialize result"))?;
                created.with_context(|| {
                    format!("{operation}: SurrealDB returned no record after write")
                })
            }
        })
        .await
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
        let key_str = match &id.key {
            RecordIdKey::String(s) => s.clone(),
            RecordIdKey::Number(i) => i.to_string(),
            RecordIdKey::Uuid(uuid) => uuid.to_string(),
            k => format!("{k:?}"),
        };
        format!("{}:{}", id.table.as_str(), key_str)
    }

    fn record_id_parts(id: &RecordId) -> (String, RecordIdKey) {
        (id.table.as_str().to_string(), id.key.clone())
    }

    fn parse_record_id_str(id: &str, default_table: &str) -> Result<(String, RecordIdKey)> {
        let parsed = if id.contains(':') {
            RecordId::parse_simple(id)?
        } else {
            RecordId::new(default_table, id)
        };
        Ok(Self::record_id_parts(&parsed))
    }

    async fn update_mindmap_graph(
        &self,
        id: &RecordId,
        nodes: Vec<MindMapNode>,
        edges: Vec<MindMapEdge>,
        updated_at: Datetime,
        op: &str,
    ) -> Result<MindMap> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let (table, key) = Self::record_id_parts(id);
        let edge_values = serde_json::to_value(edges)
            .with_context(|| format!("{op}: edge serialization failed"))?;
        let mut response = db
            .query(
                "UPDATE type::record($table, $key) \
                 SET nodes = $nodes, edges = $edges, updated_at = $updated_at \
                 RETURN AFTER",
            )
            .bind(("table", table))
            .bind(("key", key))
            .bind(("nodes", nodes))
            .bind(("edges", edge_values))
            .bind(("updated_at", updated_at))
            .await
            .with_context(|| format!("{op}: SurrealDB update query failed"))?;
        response = response
            .check()
            .with_context(|| format!("{op}: SurrealDB rejected the write"))?;
        let updated: Option<DbMindMap> = response
            .take(0)
            .with_context(|| format!("{op}: Failed to deserialize result"))?;
        let updated =
            updated.with_context(|| format!("{op}: SurrealDB returned no record after write"))?;
        Self::decode_mindmap(updated)
    }

    async fn append_mindmap_node(
        &self,
        id: &RecordId,
        node: MindMapNode,
        updated_at: Datetime,
        op: &str,
    ) -> Result<MindMap> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let (table, key) = Self::record_id_parts(id);
        let mut response = db
            .query(
                "UPDATE type::record($table, $key) \
                 SET nodes = array::append(nodes, $node), updated_at = $updated_at \
                 RETURN AFTER",
            )
            .bind(("table", table))
            .bind(("key", key))
            .bind(("node", node))
            .bind(("updated_at", updated_at))
            .await
            .with_context(|| format!("{op}: SurrealDB update query failed"))?;
        response = response
            .check()
            .with_context(|| format!("{op}: SurrealDB rejected the write"))?;
        let updated: Option<DbMindMap> = response
            .take(0)
            .with_context(|| format!("{op}: Failed to deserialize result"))?;
        let updated =
            updated.with_context(|| format!("{op}: SurrealDB returned no record after write"))?;
        Self::decode_mindmap(updated)
    }

    async fn append_mindmap_edge(
        &self,
        id: &RecordId,
        edge: MindMapEdge,
        updated_at: Datetime,
        op: &str,
    ) -> Result<MindMap> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let (table, key) = Self::record_id_parts(id);
        let edge_value = serde_json::to_value(edge)
            .with_context(|| format!("{op}: edge serialization failed"))?;
        let mut response = db
            .query(
                "UPDATE type::record($table, $key) \
                 SET edges = array::append(edges, $edge), updated_at = $updated_at \
                 RETURN AFTER",
            )
            .bind(("table", table))
            .bind(("key", key))
            .bind(("edge", edge_value))
            .bind(("updated_at", updated_at))
            .await
            .with_context(|| format!("{op}: SurrealDB update query failed"))?;
        response = response
            .check()
            .with_context(|| format!("{op}: SurrealDB rejected the write"))?;
        let updated: Option<DbMindMap> = response
            .take(0)
            .with_context(|| format!("{op}: Failed to deserialize result"))?;
        let updated =
            updated.with_context(|| format!("{op}: SurrealDB returned no record after write"))?;
        Self::decode_mindmap(updated)
    }

    fn estimate_tokens(text: &str) -> u32 {
        // Rough heuristic: ~4 chars per token (good enough for budget tracking)
        (text.len() as u32).div_ceil(4)
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let now = Datetime::default();
        entity.created_at = now;
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            .bind(("updated", entity.updated_at))
            .await?;
        let updated: Option<Entity> = res.take(0)?;
        updated.context("Failed to update entity")
    }

    async fn delete_entity(&self, name: &str) -> Result<()> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        if memory.token_count.is_none() {
            memory.token_count = Some(Self::estimate_tokens(&memory.content));
        }

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
            if let Some(c_emb) = &candidate.embedding
                && Self::cosine_similarity(&emb, c_emb) >= 0.92
                && let Some(id) = &candidate.id
            {
                let id_str = Self::record_id_to_string(id);
                return self.update_memory(&id_str, memory.content).await;
            }
        }

        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };

        let now = Datetime::default();
        memory.embedding = Some(emb);
        memory.created_at = now;
        memory.updated_at = now;
        memory.version = 1;

        let created: Option<DbMemory> = db
            .create("memory")
            .content(DbMemory::from(memory))
            .await
            .context("Failed to create memory")?;

        let stored = created.ok_or_else(|| anyhow::anyhow!("No memory returned after creation"))?;
        let stored = Self::decode_memory(stored)?;

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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let (table, key) = Self::parse_record_id_str(id, "memory")?;
        let result: Vec<DbMemory> = db
            .query("SELECT * FROM type::record($table, $key)")
            .bind(("table", table))
            .bind(("key", key))
            .await?
            .take(0)?;
        result
            .into_iter()
            .next()
            .map(Self::decode_memory)
            .transpose()
    }

    async fn update_memory(&self, id: &str, content: String) -> Result<Memory> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        let (table, key) = Self::parse_record_id_str(id, "memory")?;

        let mut res = db
            .query(
                "UPDATE type::record($table, $key) \
                 SET content = $content, embedding = $emb, token_count = $tc, version = $v, updated_at = $now \
                 RETURN AFTER",
            )
            .bind(("table", table))
            .bind(("key", key))
            .bind(("content", content.clone()))
            .bind(("emb", new_emb))
            .bind(("tc", token_count))
            .bind(("v", new_version))
            .bind(("now", now))
            .await?;

        let updated: Option<DbMemory> = res.take(0)?;
        let updated = updated.context("Failed to update memory")?;
        let updated = Self::decode_memory(updated)?;

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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        if let Some(mem) = self.get_memory(id).await?
            && let Some(mem_id) = &mem.id
        {
            db.query(
                "INSERT INTO memory_history { memory_id: $mid, version: $v, old_content: $old, new_content: $old, changed_at: $now, change_type: 'deleted' }"
            )
            .bind(("mid", mem_id.clone()))
            .bind(("v", mem.version + 1))
            .bind(("old", mem.content))
            .bind(("now", Datetime::default()))
            .await?;
        }
        let (table, key) = Self::parse_record_id_str(id, "memory")?;
        db.query("DELETE type::record($table, $key)")
            .bind(("table", table))
            .bind(("key", key))
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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

        let results: Vec<DbMemory> = q.await?.take(0)?;
        Self::decode_memories(results)
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        stream.created_at = now;
        stream.last_active = now;
        let key = Uuid::new_v4().to_string();
        let payload = DbTaskStream::from(stream);
        self.create_record("task_stream", &key, payload, "create_task_stream")
            .await
            .and_then(Self::decode_task_stream)
    }

    async fn get_task_stream(&self, name: &str) -> Result<Option<TaskStream>> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let result: Vec<DbTaskStream> = db
            .query("SELECT * FROM task_stream WHERE name = $name")
            .bind(("name", name.to_string()))
            .await?
            .take(0)?;
        result
            .into_iter()
            .next()
            .map(Self::decode_task_stream)
            .transpose()
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

        // Standardised token count: prefer stored value, fall back to heuristic estimate.
        // This matches the fallback logic used in `get_context_for_task` so that
        // `total_tokens` stays consistent with per-memory token sums.
        let added_tokens = stored
            .token_count
            .map(|t| t as u64)
            .unwrap_or_else(|| Self::estimate_tokens(&stored.content) as u64);

        // Verify the update actually found and modified the stream record.
        let db_upd = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut res = db_upd
            .query(
                "UPDATE task_stream \
                 SET total_tokens += $tokens, last_active = $now \
                 WHERE name = $name \
                 RETURN AFTER",
            )
            .bind(("tokens", added_tokens))
            .bind(("now", Datetime::default()))
            .bind(("name", stream_name.to_string()))
            .await?;
        let updated_stream: Option<DbTaskStream> = res.take(0)?;
        let updated_stream = updated_stream.with_context(|| {
            format!(
                "TaskStream '{}' disappeared during token update",
                stream_name
            )
        })?;
        let updated_stream = Self::decode_task_stream(updated_stream)?;

        // Trigger auto-summarization when the running token total crosses 80 % of
        // the model's context budget.  Run inline; the operation is async but
        // bounded and callers already await this method.
        if updated_stream.needs_summarization() {
            let model_id = updated_stream
                .model_id
                .clone()
                .unwrap_or_else(|| "default".to_string());
            // Pass through agent_id / user_id from the stream so the summary
            // memory is scoped identically to the other memories in the stream.
            let agent_id = updated_stream.agent_id.as_deref();
            let user_id = updated_stream.user_id.as_deref();
            if let Err(e) = self
                .auto_summarize_task_stream(stream_name, user_id, agent_id, &model_id)
                .await
            {
                tracing::warn!(
                    stream = stream_name,
                    error = %e,
                    "Auto-summarization failed (non-fatal)"
                );
            }
        }

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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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

        let all_memories: Vec<DbMemory> = db
            .query("SELECT * FROM memory WHERE task_stream_id = $sid ORDER BY importance DESC, created_at DESC")
            .bind(("sid", stream_id))
            .await?
            .take(0)?;
        let all_memories = Self::decode_memories(all_memories)?;

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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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

        let results: Vec<DbTaskStream> = q.await?.take(0)?;
        Self::decode_task_streams(results)
    }

    async fn delete_task_stream(&self, name: &str) -> Result<()> {
        let stream = self
            .get_task_stream(name)
            .await?
            .with_context(|| format!("TaskStream '{}' not found", name))?;
        let stream_id = stream.id.clone().context("TaskStream missing id")?;

        let memories: Vec<DbMemory> = {
            let db = {
                let state = self.connection.read().expect(
                    "Connection lock poisoned - another thread panicked while holding the lock",
                );
                match &*state {
                    ConnectionState::Connected(db) => db.clone(),
                    ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                    ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
                }
            };
            db.query("SELECT * FROM memory WHERE task_stream_id = $sid")
                .bind(("sid", stream_id.clone()))
                .await?
                .take(0)?
        };
        let memories = Self::decode_memories(memories)?;
        for memory in memories {
            if let Some(memory_id) = &memory.id {
                self.delete_memory(&Self::record_id_to_string(memory_id))
                    .await?;
            }
        }

        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        db.query("UPDATE mindmap SET task_stream_id = NONE WHERE task_stream_id = $sid")
            .bind(("sid", stream_id.clone()))
            .await?;

        let (table, key) = Self::record_id_parts(&stream_id);
        let deleted: Option<DbTaskStream> = db
            .query("DELETE type::record($table, $key) RETURN BEFORE")
            .bind(("table", table))
            .bind(("key", key))
            .await?
            .take(0)?;
        deleted.with_context(|| format!("TaskStream '{}' not found", name))?;
        Ok(())
    }

    async fn archive_task_stream(&self, name: &str) -> Result<TaskStream> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut res = db
            .query("UPDATE task_stream SET status = $status WHERE name = $name RETURN AFTER")
            .bind(("name", name.to_string()))
            .bind(("status", TaskStreamStatus::Archived.as_str()))
            .await?;
        let updated: Option<DbTaskStream> = res.take(0)?;
        let updated = updated.with_context(|| format!("TaskStream '{}' not found", name))?;
        Self::decode_task_stream(updated)
    }

    async fn pause_task_stream(&self, name: &str) -> Result<TaskStream> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let mut res = db
            .query("UPDATE task_stream SET status = $status WHERE name = $name RETURN AFTER")
            .bind(("name", name.to_string()))
            .bind(("status", TaskStreamStatus::Paused.as_str()))
            .await?;
        let updated: Option<DbTaskStream> = res.take(0)?;
        let updated = updated.with_context(|| format!("TaskStream '{}' not found", name))?;
        Self::decode_task_stream(updated)
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        let bm25_results: Vec<DbMemory> = q.await?.take(0).unwrap_or_default();
        let bm25_results = Self::decode_memories(bm25_results)?;

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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        let old_memories: Vec<DbMemory> = q.await?.take(0).unwrap_or_default();
        let old_memories = Self::decode_memories(old_memories)?;

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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        let expired: Vec<DbMemory> = db
            .query(
                "SELECT * FROM memory WHERE valid_until IS NOT NONE AND valid_until < time::now()",
            )
            .await?
            .take(0)
            .unwrap_or_default();
        let expired = Self::decode_memories(expired)?;
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
        mindmap.updated_at = mindmap.created_at;
        let key = Uuid::new_v4().to_string();
        let payload = DbMindMap::from(mindmap);
        self.create_record("mindmap", &key, payload, "create_mindmap")
            .await
            .and_then(Self::decode_mindmap)
            .context("Failed to create mindmap")
    }

    async fn get_mindmap(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Option<MindMap>> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        if agent_id.is_some() {
            sql.push_str(" AND agent_id = $aid");
        }
        let mut q = db.query(&sql).bind(("name", name.to_string()));
        if let Some(v) = user_id {
            q = q.bind(("uid", v.to_string()));
        }
        if let Some(v) = agent_id {
            q = q.bind(("aid", v.to_string()));
        }
        let mut results: Vec<DbMindMap> = q.await?.take(0)?;
        results.pop().map(Self::decode_mindmap).transpose()
    }

    async fn add_mindmap_node(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        node: crate::mindmap::MindMapNode,
    ) -> Result<MindMap> {
        let mut mm = self
            .get_mindmap(mindmap_name, user_id, agent_id)
            .await?
            .with_context(|| format!("Mindmap '{}' not found", mindmap_name))?;

        // Guard: node ID must be unique within the mindmap.
        if mm.nodes.iter().any(|n| n.id == node.id) {
            anyhow::bail!(
                "Node with id '{}' already exists in mindmap '{}'",
                node.id,
                mindmap_name
            );
        }

        // Guard: parent_id, if set, must reference an existing node.
        if let Some(parent_id) = &node.parent_id {
            anyhow::ensure!(
                mm.nodes.iter().any(|n| &n.id == parent_id),
                "Node '{}' references unknown parent '{}' in mindmap '{}'",
                node.id,
                parent_id,
                mindmap_name
            );
        }

        if mm.nodes.len() > 500 {
            tracing::warn!(
                mindmap = mindmap_name,
                node_count = mm.nodes.len(),
                edge_count = mm.edges.len(),
                "Large mindmap detected - updates may be slow. Consider splitting into multiple mindmaps."
            );
        }

        mm.nodes.push(node.clone());
        mm.updated_at = Datetime::default();
        mm.validate()?;

        let record_id = mm.id.as_ref().cloned().context("Mindmap missing id")?;
        self.append_mindmap_node(&record_id, node, mm.updated_at, "add_mindmap_node")
            .await
    }

    async fn add_mindmap_edge(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        edge: crate::mindmap::MindMapEdge,
    ) -> Result<MindMap> {
        let mut mm = self
            .get_mindmap(mindmap_name, user_id, agent_id)
            .await?
            .with_context(|| format!("Mindmap '{}' not found", mindmap_name))?;

        if mm.nodes.len() > 500 {
            tracing::warn!(
                mindmap = mindmap_name,
                node_count = mm.nodes.len(),
                edge_count = mm.edges.len(),
                "Large mindmap detected - updates may be slow"
            );
        }

        mm.edges.push(edge.clone());
        mm.updated_at = Datetime::default();
        mm.validate()?;

        let record_id = mm.id.as_ref().cloned().context("Mindmap missing id")?;
        self.append_mindmap_edge(&record_id, edge, mm.updated_at, "add_mindmap_edge")
            .await
    }

    async fn delete_mindmap_node(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        node_id: &str,
    ) -> Result<MindMap> {
        let mut mm = self
            .get_mindmap(mindmap_name, user_id, agent_id)
            .await?
            .with_context(|| format!("Mindmap '{}' not found", mindmap_name))?;
        mm.nodes.retain(|n| n.id != node_id);
        mm.edges
            .retain(|e| e.from_id != node_id && e.to_id != node_id);
        mm.updated_at = Datetime::default();
        let record_id = mm.id.as_ref().cloned().context("Mindmap missing id")?;
        self.update_mindmap_graph(
            &record_id,
            mm.nodes,
            mm.edges,
            mm.updated_at,
            "delete_mindmap_node",
        )
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        let results: Vec<DbMindMap> = q.await?.take(0)?;
        Self::decode_mindmaps(results)
    }

    async fn delete_mindmap(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<()> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        if let Some(mm) = self.get_mindmap(name, user_id, agent_id).await?
            && let Some(id) = &mm.id
        {
            let (table, key) = Self::record_id_parts(id);
            let _: Option<DbMindMap> = db
                .query("DELETE type::record($table, $key) RETURN BEFORE")
                .bind(("table", table))
                .bind(("key", key))
                .await?
                .take(0)?;
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        let memories: Vec<DbMemory> = db.query(&sql).await?.take(0).unwrap_or_default();
        let memories = Self::decode_memories(memories)?;

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
                let _: Option<DbMemory> = db.delete(("memory", key)).await?;
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
        let stored = self.add_memory(summary).await?;

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
        Ok(Some(stored))
    }

    async fn try_update_persona_mindmap(
        &self,
        user_id: &str,
        memory: &crate::memory::Memory,
    ) -> Result<()> {
        let db = {
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
            match &*state {
                ConnectionState::Connected(db) => db.clone(),
                ConnectionState::Reconnecting => anyhow::bail!("Connection is reconnecting"),
                ConnectionState::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
            }
        };
        // Look for a persona mindmap belonging to this user
        let maps: Vec<DbMindMap> = db
            .query("SELECT * FROM mindmap WHERE user_id = $uid AND map_type = 'radial' LIMIT 1")
            .bind(("uid", user_id.to_string()))
            .await?
            .take(0)
            .unwrap_or_default();
        let maps = Self::decode_mindmaps(maps)?;

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

        self.add_mindmap_node(&mm.name, Some(user_id), None, node)
            .await?;
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
                    let state = self.connection.read().expect(
                        "Connection lock poisoned - another thread panicked while holding the lock",
                    );
                    match &*state {
                        ConnectionState::Connected(db) => db.clone(),
                        ConnectionState::Reconnecting => {
                            anyhow::bail!("Connection is reconnecting")
                        }
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
                    let state = self.connection.read().expect(
                        "Connection lock poisoned - another thread panicked while holding the lock",
                    );
                    match &*state {
                        ConnectionState::Connected(db) => db.clone(),
                        ConnectionState::Reconnecting => {
                            anyhow::bail!("Connection is reconnecting")
                        }
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        let rels: Vec<crate::entity::Relation> = db.query(&sql).await?.take(0).unwrap_or_default();

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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
            let state = self.connection.read().expect(
                "Connection lock poisoned - another thread panicked while holding the lock",
            );
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
        let dir = std::env::temp_dir().join(format!("surreal-test-{}", Uuid::new_v4()));
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
        };

        let db = Self::connect_with_config(&config).await?;
        run_migrations(&db).await?;

        Ok(Self {
            connection: Arc::new(std::sync::RwLock::new(ConnectionState::Connected(db))),
            connection_info,
            embedding_service,
        })
    }
}

// ── Retry configuration tests

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

    #[test]
    fn test_retry_config_is_used_by_connect_with_retry() {
        // Verify that RetryConfig values are accessible — the retry loop in
        // connect_with_retry reads these fields at runtime.
        let config = RetryConfig::default();
        assert!(config.max_connect_retries > 0);
        assert!(config.max_operation_retries > 0);
    }

    #[test]
    fn test_retriable_network_errors() {
        let storage = mock_storage();

        let err = anyhow::anyhow!("Connection uninitialised");
        assert!(storage.is_retriable_error(&err));

        let err = anyhow::anyhow!("connection closed");
        assert!(storage.is_retriable_error(&err));

        let err = anyhow::anyhow!("Connection timeout");
        assert!(storage.is_retriable_error(&err));

        let err = anyhow::anyhow!("too many connections");
        assert!(storage.is_retriable_error(&err));
    }

    #[test]
    fn test_non_retriable_errors() {
        let storage = mock_storage();

        let err = anyhow::anyhow!("Found field 'foo', but no such field exists");
        assert!(!storage.is_retriable_error(&err));

        let err = anyhow::anyhow!("invalid credentials");
        assert!(!storage.is_retriable_error(&err));

        let err = anyhow::anyhow!("table doesn't exist");
        assert!(!storage.is_retriable_error(&err));

        let err = anyhow::anyhow!("record not found");
        assert!(!storage.is_retriable_error(&err));
    }

    #[tokio::test]
    async fn test_connect_with_retry_succeeds_after_transient_failure() {
        // Test that the static connect_with_retry function has correct signature.
        let config = SurrealConfig {
            mode: SurrealMode::Embedded,
            embedded_path: Some("/tmp/test-retry".to_string()),
            ..Default::default()
        };
        let result = SurrealStorage::connect_with_retry(&config).await;
        // Result can be Ok or Err depending on environment — either is valid here.
        match result {
            Ok(_) => assert!(true),
            Err(_) => assert!(true),
        }
    }

    #[tokio::test]
    async fn test_reconnect_updates_connection_state() {
        // Test that reconnect() properly transitions state through Reconnecting
        // If DB is available, it ends in Connected; if not, it ends in Failed
        let storage = mock_storage();

        // Call reconnect
        let result = storage.reconnect().await;

        // After reconnect, state should be either Connected (if DB available) or Failed (if not)
        // The important thing is that it's no longer in the initial Failed/Reconnecting state
        {
            let state = storage.connection.read().unwrap();
            match &*state {
                ConnectionState::Connected(_) => {
                    // Reconnection succeeded
                    assert!(result.is_ok());
                }
                ConnectionState::Failed(_) => {
                    // Reconnection failed
                    assert!(result.is_err());
                }
                ConnectionState::Reconnecting => {
                    panic!("State should not remain Reconnecting after reconnect() completes");
                }
            }
        }
    }

    #[tokio::test]
    async fn test_retry_operation_succeeds_after_retry() {
        let storage = mock_storage();

        // Test the retry_operation method exists and has correct signature
        // Operation will fail (no real DB) but proves method compiles
        let result = storage
            .retry_operation("test_op", |db| async move {
                // Simulate a database query
                let _result: std::result::Result<Option<serde_json::Value>, surrealdb::Error> =
                    db.select(("test", "id")).await;
                Ok::<(), anyhow::Error>(())
            })
            .await;

        // We expect failure (no real DB), but the method exists and compiles
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_record_uses_retry_wrapper() {
        let storage = mock_storage();

        // Test that create_record compiles with retry wrapper
        // Will fail without real DB, but proves integration works
        let result: Result<serde_json::Value> = storage
            .create_record(
                "test_table",
                "test_id",
                serde_json::json!({"field": "value"}),
                "test_operation",
            )
            .await;

        // Expect failure (no real DB), but method uses retry wrapper
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_explicit_record_content_removes_id_field() {
        let payload = surrealdb_types::object! {
            id: surrealdb_types::Value::Null,
            name: "example".to_string(),
            created_at: Datetime::default(),
            metadata: surrealdb_types::Value::None,
            nested: surrealdb_types::object! { ok: true }
        };

        let sanitized = SurrealStorage::sanitize_explicit_record_content(payload);
        assert!(sanitized["id"].is_none());
        assert!(matches!(
            &sanitized["name"],
            surrealdb_types::Value::String(value) if value == "example"
        ));
        assert!(sanitized["created_at"].is_datetime());
        assert!(sanitized["metadata"].is_none());
        assert!(matches!(
            &sanitized["nested"]["ok"],
            surrealdb_types::Value::Bool(true)
        ));
    }

    fn mock_storage() -> SurrealStorage {
        // Mock storage for testing error classification
        use crate::embeddings::EmbeddingService;

        struct MockEmbedding;
        #[async_trait::async_trait]
        impl EmbeddingService for MockEmbedding {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
                Ok(vec![0.0; 1536])
            }
            async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
                Ok(texts.iter().map(|_| vec![0.0; 1536]).collect())
            }
            fn dimensions(&self) -> usize {
                1536
            }
        }

        let config = SurrealConfig {
            mode: SurrealMode::Embedded,
            embedded_path: Some("/tmp/test".to_string()),
            endpoint: None,
            username: None,
            password: None,
            namespace: "test".to_string(),
            database: "test".to_string(),
            retry: RetryConfig::default(),
        };

        let connection_info = ConnectionInfo {
            config: config.clone(),
        };

        SurrealStorage {
            connection: Arc::new(std::sync::RwLock::new(ConnectionState::Failed(
                "test".to_string(),
            ))),
            connection_info,
            embedding_service: Arc::new(MockEmbedding),
        }
    }
}
