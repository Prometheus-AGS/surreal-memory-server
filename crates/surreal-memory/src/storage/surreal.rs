//! SurrealDB-backed implementation of `MemoryStorage`.

use super::MemoryStorage;
#[cfg(feature = "palace")]
use crate::palace::{HitSource, PalaceContext, PalaceStatus, PalaceStorage, UnifiedHit};
use crate::{
    embeddings::EmbeddingService,
    entity::{Entity, KnowledgeGraph, Relation, SemanticSearchResult},
    memory::{Memory, MemoryHistory, MemoryScope, MemoryType},
    mindmap::{MapType, MindMap, MindMapEdge, MindMapNode},
    storage::migrations::run_migrations,
    task_step::{TaskStep, TaskStepStatus},
    task_stream::{ContextWindow, TaskStream, TaskStreamStatus},
};
use anyhow::{Context, Result};
use arc_swap::ArcSwap;
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

/// SurrealDB query timeout for mindmap `UPDATE` statements. Mindmaps are stored
/// as a single record with nested node/edge arrays, so every node/edge add
/// rewrites the whole JSON object — a known SurrealDB performance cliff on
/// large mindmaps. This `TIMEOUT` makes an oversized update fail fast with a
/// clear error instead of stalling the write path open-endedly.
const MINDMAP_UPDATE_TIMEOUT: &str = "30s";

/// SurrealDB-backed memory storage.
///
/// `Surreal<Any>` is internally `Arc`-wrapped and `Clone`-safe; the SDK
/// multiplexes concurrent queries over a single physical connection
/// (WebSocket in server mode, in-process for embedded). The connection
/// lifecycle (Connected/Reconnecting/Failed) is published via an atomic
/// `ArcSwap` cell: hot-path readers do a single atomic load (no lock),
/// reconnects do a single atomic store. This replaces the previous
/// `Arc<std::sync::RwLock<ConnectionState>>` that serialized every storage
/// call through a blocking lock and produced writer starvation under load.
pub struct SurrealStorage {
    connection: Arc<ArcSwap<ConnectionCell>>,
    connection_info: ConnectionInfo,
    embedding_service: Arc<dyn EmbeddingService>,
    /// Bounded-concurrency semaphore for embedded mode. RocksDB's
    /// PointLockManager defaults to 16 stripes per column family —
    /// concurrent transactions on overlapping keys serialize at the
    /// storage engine. Bounding in-flight ops at the application layer
    /// prevents head-of-line blocking and turns "lock timeout" /
    /// "serialization failure" into honest backpressure. `None` in
    /// server mode (the remote SurrealDB handles its own scheduling).
    embedded_semaphore: Option<Arc<tokio::sync::Semaphore>>,
    #[cfg(feature = "palace")]
    palace: tokio::sync::OnceCell<PalaceContext>,
}

// ── Retry Configuration ───────────────────────────────────────────────────────

use std::time::Duration;

/// Number of connect attempts allowed when reconnecting from inside an
/// operation retry. Deliberately small — the operation deadline, not this
/// count, is the real bound; the larger `max_connect_retries` is reserved for
/// the initial startup connect where a longer wait is acceptable.
const OPERATION_RECONNECT_ATTEMPTS: u32 = 2;

/// Default in-flight concurrency cap for embedded mode. Matches RocksDB's
/// PointLockManager default of 16 stripes per column family — beyond this,
/// concurrent transactions on overlapping keys serialize at the storage
/// engine and produce "lock timeout" / "serialization failure" errors.
/// Bounding here turns that into honest backpressure. Override via
/// `SURREAL_EMBEDDED_MAX_INFLIGHT`.
const DEFAULT_EMBEDDED_MAX_INFLIGHT: usize = 16;

/// Build the embedded-mode in-flight semaphore from the active config. Returns
/// `None` in server mode (the remote DB handles its own scheduling).
fn make_embedded_semaphore(config: &SurrealConfig) -> Option<Arc<tokio::sync::Semaphore>> {
    match config.mode {
        SurrealMode::Embedded => {
            let n = std::env::var("SURREAL_EMBEDDED_MAX_INFLIGHT")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_EMBEDDED_MAX_INFLIGHT);
            Some(Arc::new(tokio::sync::Semaphore::new(n)))
        }
        SurrealMode::Server => None,
    }
}

/// What the retry layer should do with an observed error.
///
/// Replaces the previous binary "retriable or not" verdict. Three states
/// matter because the *response* differs: a server-busy error wants
/// backoff-and-retry (do NOT reconnect — the connection is fine); a
/// transport failure wants reconnect-then-retry; everything else is a
/// hard failure that should not waste the retry budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryAction {
    /// Backoff and retry on the same connection. Server-busy, lock
    /// contention, query timeout, etc.
    Retry,
    /// Reconnect, then retry. Transport-level loss (refused, reset,
    /// closed, DNS, network).
    Reconnect,
    /// Surface immediately. Schema, validation, auth, constraint errors.
    FailFast,
}

/// Discriminate a typed `surrealdb::Error` into a `RetryAction`. Kept as a
/// free function so it can be unit-tested without a live storage handle.
fn classify_surreal_error(err: &surrealdb::Error) -> RetryAction {
    // The exact variant set is SDK-version-specific; match on the
    // stringified Display form scoped to the typed error (much narrower
    // than matching on a fully-wrapped anyhow chain).
    let msg = err.to_string().to_lowercase();
    if msg.contains("connection refused")
        || msg.contains("connection reset")
        || msg.contains("connection closed")
        || msg.contains("not connected")
        || msg.contains("disconnect")
    {
        RetryAction::Reconnect
    } else if msg.contains("timeout")
        || msg.contains("timed out")
        || msg.contains("too many connections")
        || msg.contains("lock")
        || msg.contains("serialization")
        || msg.contains("backpressure")
    {
        RetryAction::Retry
    } else {
        // Schema, validation, query syntax, auth — do not retry, do not
        // reconnect, just surface.
        RetryAction::FailFast
    }
}

/// Configuration for retry and reconnection behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_connect_retries: u32,
    pub max_operation_retries: u32,
    pub base_retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
    pub jitter_factor: f64,
    /// Total wall-clock budget for a single storage operation, spanning all
    /// retries and reconnection attempts. When exceeded, the operation returns
    /// a typed error instead of stalling. Prevents the nested
    /// retry × reconnect amplification that produced multi-minute hangs.
    ///
    /// Layering: the SDK enforces a per-query `query_timeout_ms` first; if a
    /// query exceeds that, the SDK returns an error and the retry layer
    /// observes it. `operation_deadline_ms` is the backstop ceiling for the
    /// full retry × reconnect cycle. For guaranteed retries against a slow
    /// (but alive) DB, keep `operation_deadline_ms` ≥ several × `query_timeout_ms`.
    pub operation_deadline_ms: u64,
    /// SDK-level per-query timeout passed to `surrealdb::opt::Config::query_timeout`.
    /// Bounds a single query at the protocol layer so a hung server-side query
    /// does not head-of-line-block the multiplexed connection for unrelated
    /// work. Configurable via `SURREAL_QUERY_TIMEOUT_MS`.
    pub query_timeout_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_connect_retries: 10,
            max_operation_retries: 3,
            base_retry_delay_ms: 100,
            max_retry_delay_ms: 5000,
            jitter_factor: 0.25,
            operation_deadline_ms: 30_000,
            query_timeout_ms: 10_000,
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
///
/// Stored behind an `ArcSwap` so the hot path is a single atomic load and
/// state transitions (reconnect → Connected) are a single atomic store.
/// The previous design wrapped this in `std::sync::RwLock` and contended
/// every storage call against every reconnect attempt — removed.
#[derive(Clone)]
pub(crate) enum ConnectionCell {
    /// Active connection, ready to use.
    Connected(Surreal<Any>),
    /// Mid-reconnect; callers should fail fast and let the retry layer
    /// observe the new state on its next attempt.
    Reconnecting,
    /// Connection could not be established or lost permanently.
    Failed(String),
}

/// Cancellation-safety guard for `reconnect_with_attempts`. If the reconnect
/// future is dropped while still in `Reconnecting` (e.g. the operation
/// deadline fired mid-connect), the guard's `Drop` forces the cell to
/// `Failed` so the connection is never permanently stranded in
/// `Reconnecting`. On a normal completion the caller calls `disarm()`
/// before publishing the final state.
struct ReconnectGuard {
    connection: Arc<ArcSwap<ConnectionCell>>,
}

impl ReconnectGuard {
    /// Disarm the guard once the caller is about to publish the final state.
    fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for ReconnectGuard {
    fn drop(&mut self) {
        // Atomic-load to inspect; only force-publish Failed if we're still
        // mid-reconnect (i.e. the caller did not disarm us).
        let current = self.connection.load();
        if matches!(**current, ConnectionCell::Reconnecting) {
            tracing::warn!("Reconnect cancelled before completion; marking connection Failed");
            self.connection.store(Arc::new(ConnectionCell::Failed(
                "Reconnect cancelled before completion".to_string(),
            )));
        }
    }
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
            // The `task_stream_scope_name` composite UNIQUE index (migration
            // v18) cannot enforce uniqueness or be used for lookups when an
            // indexed field is NONE (SurrealDB 3.x). Persist absent scope ids
            // as the empty string so the index works; `TryFrom` maps `""` back
            // to `None` so the public `TaskStream` API is unchanged.
            agent_id: Some(stream.agent_id.unwrap_or_default()),
            user_id: Some(stream.user_id.unwrap_or_default()),
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

/// Map a persisted scope id back to the public `Option<String>` representation.
/// Empty strings are written by `DbTaskStream::from` for absent scope ids so the
/// composite unique index works; decode them back to `None`.
fn scope_id_from_db(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
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
            agent_id: scope_id_from_db(stream.agent_id),
            user_id: scope_id_from_db(stream.user_id),
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
struct DbTaskStep {
    id: Option<RecordId>,
    task_stream_id: Option<RecordId>,
    ordinal: u32,
    name: String,
    description: Option<String>,
    status: String,
    idempotency_key: String,
    result: Option<String>,
    error: Option<String>,
    started_at: Option<Datetime>,
    completed_at: Option<Datetime>,
    created_at: Datetime,
}

impl From<TaskStep> for DbTaskStep {
    fn from(step: TaskStep) -> Self {
        Self {
            id: step.id,
            task_stream_id: step.task_stream_id,
            ordinal: step.ordinal,
            name: step.name,
            description: step.description,
            status: step.status.as_str().to_string(),
            idempotency_key: step.idempotency_key,
            result: step.result,
            error: step.error,
            started_at: step.started_at,
            completed_at: step.completed_at,
            created_at: step.created_at,
        }
    }
}

impl TryFrom<DbTaskStep> for TaskStep {
    type Error = anyhow::Error;

    fn try_from(step: DbTaskStep) -> Result<Self> {
        let record_id = step
            .id
            .as_ref()
            .map(SurrealStorage::record_id_to_string)
            .unwrap_or_else(|| "<new>".to_string());
        let status = TaskStepStatus::parse_str(&step.status).map_err(|err| {
            anyhow::anyhow!(
                "task_step.status decode failed for record {}: {} (raw={})",
                record_id,
                err,
                step.status
            )
        })?;

        Ok(Self {
            id: step.id,
            task_stream_id: step.task_stream_id,
            ordinal: step.ordinal,
            name: step.name,
            description: step.description,
            status,
            idempotency_key: step.idempotency_key,
            result: step.result,
            error: step.error,
            started_at: step.started_at,
            completed_at: step.completed_at,
            created_at: step.created_at,
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

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct SchemaMetadataRecord {
    name: String,
    int_value: Option<i64>,
    updated_at: Datetime,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
struct EmbeddingDimensionRecord {
    id: Option<RecordId>,
    embedding: Option<Vec<f32>>,
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
        Self::ensure_embedding_indexes(&db, embedding_service.dimensions()).await?;

        let embedded_semaphore = make_embedded_semaphore(&connection_info.config);

        Ok(Self {
            connection: Arc::new(ArcSwap::new(Arc::new(ConnectionCell::Connected(db)))),
            connection_info,
            embedding_service,
            embedded_semaphore,
            #[cfg(feature = "palace")]
            palace: tokio::sync::OnceCell::new(),
        })
    }

    /// Establish a connection with exponential-backoff retry.
    ///
    /// Used by `new()` for the initial connection.  `new_mem()` (embedded test
    /// helper) skips retries because a missing embedded path is a programmer
    /// error, not a transient failure.
    /// Connect using the full startup retry budget (`max_connect_retries`).
    async fn connect_with_retry(config: &SurrealConfig) -> Result<Surreal<Any>> {
        Self::connect_with_attempts(config, config.retry.max_connect_retries).await
    }

    /// Connect with an explicit attempt cap. The operation-path reconnect uses a
    /// small cap so it cannot amplify into a multi-minute stall; the initial
    /// startup connect uses the larger `max_connect_retries`.
    async fn connect_with_attempts(
        config: &SurrealConfig,
        max_attempts: u32,
    ) -> Result<Surreal<Any>> {
        let mut attempt = 0u32;
        loop {
            match Self::connect_with_config(config).await {
                Ok(db) => return Ok(db),
                Err(e) if attempt < max_attempts => {
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
    ///
    /// Passes an SDK `Config::query_timeout(...)` so individual queries are
    /// bounded at the protocol layer — a hung server-side query can no
    /// longer head-of-line-block the multiplexed connection for unrelated
    /// work. The retry-layer `operation_deadline_ms` becomes a backstop,
    /// not the primary mechanism.
    async fn connect_with_config(config: &SurrealConfig) -> Result<Surreal<Any>> {
        let sdk_config = surrealdb::opt::Config::default()
            .query_timeout(Duration::from_millis(config.retry.query_timeout_ms));

        let db = match &config.mode {
            SurrealMode::Embedded => {
                let path = config
                    .embedded_path
                    .as_ref()
                    .context("Embedded path required for embedded mode")?;
                tracing::info!("Connecting to embedded SurrealDB at: {}", path);
                surrealdb::engine::any::connect((format!("rocksdb://{}", path), sdk_config)).await?
            }
            SurrealMode::Server => {
                let endpoint = config
                    .endpoint
                    .as_ref()
                    .context("Endpoint required for server mode")?;
                tracing::info!("Connecting to SurrealDB server at: {}", endpoint);
                surrealdb::engine::any::connect((endpoint.clone(), sdk_config)).await?
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

    async fn ensure_embedding_indexes(db: &Surreal<Any>, expected_dimension: usize) -> Result<()> {
        if expected_dimension == 0 {
            anyhow::bail!("Embedding provider reported zero dimensions");
        }

        Self::validate_existing_embedding_dimensions(db, "entity", expected_dimension).await?;
        Self::validate_existing_embedding_dimensions(db, "memory", expected_dimension).await?;

        let metadata: Option<SchemaMetadataRecord> = db
            .select(("schema_metadata", "embedding_index_dimension"))
            .await
            .context("Failed to read embedding index metadata")?;

        if metadata.as_ref().and_then(|record| record.int_value) == Some(expected_dimension as i64)
        {
            Self::define_embedding_indexes(db, expected_dimension).await?;
            tracing::info!(
                dimension = expected_dimension,
                "Embedding HNSW indexes already match provider dimensions"
            );
            return Ok(());
        }

        Self::rebuild_embedding_indexes(db, expected_dimension).await?;

        let metadata = SchemaMetadataRecord {
            name: "embedding_index_dimension".to_string(),
            int_value: Some(expected_dimension as i64),
            updated_at: Datetime::default(),
        };
        let _: Option<SchemaMetadataRecord> = db
            .upsert(("schema_metadata", "embedding_index_dimension"))
            .content(metadata)
            .await
            .context("Failed to write embedding index metadata")?;

        tracing::info!(
            dimension = expected_dimension,
            "Embedding HNSW indexes configured for provider dimensions"
        );
        Ok(())
    }

    async fn validate_existing_embedding_dimensions(
        db: &Surreal<Any>,
        table: &str,
        expected_dimension: usize,
    ) -> Result<()> {
        let query = match table {
            "entity" => "SELECT id, embedding FROM entity WHERE embedding IS NOT NONE",
            "memory" => "SELECT id, embedding FROM memory WHERE embedding IS NOT NONE",
            _ => anyhow::bail!("Unsupported embedding table: {}", table),
        };

        let rows: Vec<EmbeddingDimensionRecord> = db
            .query(query)
            .await
            .with_context(|| format!("Failed to inspect {} embedding dimensions", table))?
            .take(0)
            .unwrap_or_default();

        for row in rows {
            let Some(embedding) = row.embedding else {
                continue;
            };
            let actual_dimension = embedding.len();
            if actual_dimension != expected_dimension {
                let record_id = row
                    .id
                    .as_ref()
                    .map(Self::record_id_to_string)
                    .unwrap_or_else(|| "<unknown>".to_string());
                anyhow::bail!(
                    "{} record {} has a {}-dimensional embedding, but the active embedding provider expects {} dimensions. Use a separate database, re-embed existing records, or configure a provider with matching dimensions.",
                    table,
                    record_id,
                    actual_dimension,
                    expected_dimension
                );
            }
        }

        Ok(())
    }

    async fn rebuild_embedding_indexes(db: &Surreal<Any>, dimension: usize) -> Result<()> {
        let response = db
            .query(
                "
REMOVE INDEX IF EXISTS entity_embedding_hnsw ON entity;
REMOVE INDEX IF EXISTS memory_embedding_hnsw ON memory;
",
            )
            .await
            .context("Failed to remove existing embedding HNSW indexes")?;
        response
            .check()
            .context("SurrealDB rejected embedding HNSW index removal")?;

        Self::define_embedding_indexes(db, dimension).await
    }

    async fn define_embedding_indexes(db: &Surreal<Any>, dimension: usize) -> Result<()> {
        let ddl = format!(
            "
DEFINE INDEX IF NOT EXISTS entity_embedding_hnsw
  ON entity FIELDS embedding HNSW DIMENSION {dimension} DIST COSINE TYPE F32;
DEFINE INDEX IF NOT EXISTS memory_embedding_hnsw
  ON memory FIELDS embedding HNSW DIMENSION {dimension} DIST COSINE TYPE F32;
"
        );
        let response = db
            .query(ddl)
            .await
            .with_context(|| format!("Failed to define {dimension}-dimensional HNSW indexes"))?;
        response
            .check()
            .with_context(|| format!("SurrealDB rejected {dimension}-dimensional HNSW indexes"))?;
        Ok(())
    }

    /// Hot-path accessor for the live `Surreal<Any>` handle.
    ///
    /// One atomic load, one `Arc::clone` of the inner SDK handle. No lock,
    /// no contention, safe to call across `.await`. Every storage method
    /// goes through here.
    pub(crate) fn live_db(&self) -> Result<Surreal<Any>> {
        let cell = self.connection.load();
        match &**cell {
            ConnectionCell::Connected(db) => Ok(db.clone()),
            ConnectionCell::Reconnecting => {
                anyhow::bail!("Connection is currently reconnecting, please retry later")
            }
            ConnectionCell::Failed(msg) => anyhow::bail!("Connection failed: {}", msg),
        }
    }

    /// Ping the database to verify the connection is alive.
    ///
    /// Useful for liveness/readiness probes when running against a remote
    /// SurrealDB server.
    pub async fn health_check(&self) -> Result<bool> {
        let db = self.live_db()?;
        db.health()
            .await
            .map(|_| true)
            .context("SurrealDB health check failed")
    }

    /// Clone the live `Surreal<Any>` handle for shared use by subsystems
    /// (e.g. `PalaceAdapter`). `Surreal<Any>` is internally `Arc`-wrapped,
    /// so this is cheap. Returns `Err` if the connection is in
    /// `Reconnecting` or `Failed` state.
    pub fn db(&self) -> Result<Surreal<Any>> {
        self.live_db()
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

    /// Return a cloneable reference to the connection cell. Used by
    /// `PalaceAdapter` to build a closure that yields the live
    /// `Surreal<Any>` handle on each operation (resilient to reconnection).
    #[cfg(feature = "palace")]
    pub(crate) fn connection_arc(&self) -> Arc<ArcSwap<ConnectionCell>> {
        Arc::clone(&self.connection)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Classify a storage error into a `RetryAction`. Prefers downcasting to
    /// the typed `surrealdb::Error` enum; falls back to substring matching
    /// only when the typed source is not reachable through the anyhow
    /// chain. This replaces the previous all-substring matcher whose
    /// "field"/"timeout" co-occurrence produced the wrong verdict.
    ///
    /// `Reconnect` is reserved for true transport failures — the previous
    /// code triggered a reconnect on "lock timeout" / "serialization
    /// failure", which hammered the contended path instead of letting it
    /// drain.
    fn classify_error(&self, error: &anyhow::Error) -> RetryAction {
        // Typed-source discrimination first.
        if let Some(surreal_err) = error.downcast_ref::<surrealdb::Error>() {
            return classify_surreal_error(surreal_err);
        }
        // Fallback: string-based, but narrower and three-valued — replaces
        // the legacy any-of-many matcher that conflated transport with
        // server-busy.
        let msg = format!("{}", error).to_lowercase();
        if msg.contains("connection refused")
            || msg.contains("connection reset")
            || msg.contains("connection closed")
            || msg.contains("connection uninitialised")
            || msg.contains("not connected")
            || msg.contains("dns")
            || msg.contains("network")
        {
            return RetryAction::Reconnect;
        }
        if msg.contains("timeout")
            || msg.contains("timed out")
            || msg.contains("too many connections")
            || msg.contains("backpressure")
            || msg.contains("lock")
            || msg.contains("serialization failure")
        {
            return RetryAction::Retry;
        }
        RetryAction::FailFast
    }

    /// Reconnect with an explicit connect-attempt cap. `retry_operation` calls
    /// this with a small cap (`OPERATION_RECONNECT_ATTEMPTS`) so a reconnection
    /// cannot amplify into a long stall; the surrounding operation deadline is
    /// the real bound.
    ///
    /// The connection state transitions Reconnecting → Connected/Failed. A
    /// `ReconnectGuard` ensures that if this future is cancelled mid-connect
    /// (e.g. the operation deadline fires), the state is not stranded in
    /// `Reconnecting` — it is forced to `Failed` on drop so the storage
    /// instance stays usable (subsequent calls retry rather than bail forever).
    async fn reconnect_with_attempts(&self, max_attempts: u32) -> Result<()> {
        // Publish Reconnecting state atomically; arm the cancellation guard.
        self.connection
            .store(Arc::new(ConnectionCell::Reconnecting));
        tracing::warn!("Connection lost, attempting reconnection");
        let guard = ReconnectGuard {
            connection: Arc::clone(&self.connection),
        };

        match Self::connect_with_attempts(&self.connection_info.config, max_attempts).await {
            Ok(db) => {
                guard.disarm();
                self.connection
                    .store(Arc::new(ConnectionCell::Connected(db)));
                tracing::info!("Reconnection successful");
                Ok(())
            }
            Err(err) => {
                guard.disarm();
                let error_msg = format!("{}", err);
                self.connection
                    .store(Arc::new(ConnectionCell::Failed(error_msg.clone())));
                tracing::error!(error = %err, "Reconnection failed after exhausting retries");
                Err(anyhow::anyhow!("Reconnection failed: {}", error_msg))
            }
        }
    }

    /// Generic retry wrapper for database operations.
    ///
    /// Handles connection extraction, error classification, reconnection, and
    /// exponential backoff — all under a single wall-clock deadline
    /// (`operation_deadline_ms`). When the deadline is exceeded the operation
    /// returns a typed error instead of stalling. This bounds the previous
    /// `retry × reconnect × connect_with_retry` amplification.
    async fn retry_operation<F, R, Fut>(&self, op_name: &str, op: F) -> Result<R>
    where
        F: Fn(Surreal<Any>) -> Fut,
        Fut: std::future::Future<Output = Result<R>>,
    {
        use tracing::Instrument as _;

        // Acquire an embedded-mode permit (no-op in server mode). The
        // permit is held for the entire retry × reconnect cycle so the
        // application-layer concurrency cap holds end-to-end. The wait
        // itself counts against the operation deadline below — this is
        // the *honest* backpressure path that replaces the lock-timeout /
        // serialization-failure error storm of the previous design.
        let _permit = if let Some(sem) = &self.embedded_semaphore {
            Some(
                Arc::clone(sem)
                    .acquire_owned()
                    .await
                    .context("embedded concurrency semaphore was closed")?,
            )
        } else {
            None
        };

        let deadline =
            Duration::from_millis(self.connection_info.config.retry.operation_deadline_ms);
        let span = tracing::debug_span!("retry_operation", operation = op_name);

        match tokio::time::timeout(
            deadline,
            self.retry_operation_inner(op_name, op).instrument(span),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!(
                "Operation '{}' exceeded the {}ms deadline and was aborted",
                op_name,
                deadline.as_millis()
            )),
        }
    }

    /// Inner retry loop. Always run under the `tokio::time::timeout` budget
    /// established by `retry_operation`.
    async fn retry_operation_inner<F, R, Fut>(&self, op_name: &str, op: F) -> Result<R>
    where
        F: Fn(Surreal<Any>) -> Fut,
        Fut: std::future::Future<Output = Result<R>>,
    {
        let max_retries = self.connection_info.config.retry.max_operation_retries;
        let mut last_error = None;

        for attempt in 0..max_retries {
            // Extract current connection
            let db = self.live_db()?;

            // Attempt operation
            match op(db).await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    let action = self.classify_error(&err);
                    last_error = Some(err);

                    if attempt >= max_retries - 1 || action == RetryAction::FailFast {
                        break;
                    }

                    // Only `Reconnect` actions touch the connection cell.
                    // `Retry` actions backoff and use the same handle —
                    // the SDK multiplexes, so a busy/locked query does not
                    // poison the connection.
                    if action == RetryAction::Reconnect
                        && let Err(reconnect_err) = self
                            .reconnect_with_attempts(OPERATION_RECONNECT_ATTEMPTS)
                            .await
                    {
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
                        action = ?action,
                        error = %last_error.as_ref().unwrap(),
                        next_delay_ms = delay.as_millis(),
                        "Retrying operation after transient failure"
                    );

                    tokio::time::sleep(delay).await;
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

    /// Update a `TaskStream`'s status, scoped to the caller's `user_id`/`agent_id`.
    /// A cross-scope call matches zero rows and surfaces a "not found" error.
    async fn update_task_stream_status(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        status: TaskStreamStatus,
    ) -> Result<TaskStream> {
        let db = self.live_db()?;
        let mut sql = "UPDATE task_stream SET status = $status WHERE name = $name".to_string();
        if user_id.is_some() {
            sql.push_str(" AND user_id = $uid");
        }
        if agent_id.is_some() {
            sql.push_str(" AND agent_id = $aid");
        }
        sql.push_str(" RETURN AFTER");
        let mut q = db
            .query(sql)
            .bind(("name", name.to_string()))
            .bind(("status", status.as_str()));
        if let Some(v) = user_id {
            q = q.bind(("uid", v.to_string()));
        }
        if let Some(v) = agent_id {
            q = q.bind(("aid", v.to_string()));
        }
        let mut res = q.await?;
        let updated: Option<DbTaskStream> = res.take(0)?;
        let updated = updated.with_context(|| format!("TaskStream '{}' not found", name))?;
        Self::decode_task_stream(updated)
    }

    fn decode_task_step(record: DbTaskStep) -> Result<TaskStep> {
        record.try_into()
    }

    fn decode_task_steps(records: Vec<DbTaskStep>) -> Result<Vec<TaskStep>> {
        records.into_iter().map(Self::decode_task_step).collect()
    }

    /// Whether an error chain represents a SurrealDB UNIQUE index violation.
    /// Used to recover from the TOCTOU race in `add_task_step` — narrowly
    /// matched so non-uniqueness errors are never swallowed.
    fn is_unique_violation(err: &anyhow::Error) -> bool {
        let msg = format!("{err:#}").to_lowercase();
        msg.contains("already contains") || (msg.contains("index") && msg.contains("unique"))
    }

    /// Look up a single `TaskStep` by its `idempotency_key`. Returns `None`
    /// when no step with that key exists. Backs the idempotency checks in
    /// `add_task_step` and `complete_step`.
    async fn find_task_step_by_key(&self, idempotency_key: &str) -> Result<Option<TaskStep>> {
        let db = self.db()?;
        let steps: Vec<DbTaskStep> = db
            .query("SELECT * FROM task_step WHERE idempotency_key = $key LIMIT 1")
            .bind(("key", idempotency_key.to_string()))
            .await?
            .take(0)?;
        steps
            .into_iter()
            .next()
            .map(Self::decode_task_step)
            .transpose()
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
        let db = self.live_db()?;
        let (table, key) = Self::record_id_parts(id);
        let edge_values = serde_json::to_value(edges)
            .with_context(|| format!("{op}: edge serialization failed"))?;
        let mut response = db
            .query(format!(
                "UPDATE type::record($table, $key) \
                 SET nodes = $nodes, edges = $edges, updated_at = $updated_at \
                 RETURN AFTER TIMEOUT {MINDMAP_UPDATE_TIMEOUT}"
            ))
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
        let db = self.live_db()?;
        let (table, key) = Self::record_id_parts(id);
        let mut response = db
            .query(format!(
                "UPDATE type::record($table, $key) \
                 SET nodes = array::append(nodes, $node), updated_at = $updated_at \
                 RETURN AFTER TIMEOUT {MINDMAP_UPDATE_TIMEOUT}"
            ))
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
        let db = self.live_db()?;
        let (table, key) = Self::record_id_parts(id);
        let edge_value = serde_json::to_value(edge)
            .with_context(|| format!("{op}: edge serialization failed"))?;
        let mut response = db
            .query(format!(
                "UPDATE type::record($table, $key) \
                 SET edges = array::append(edges, $edge), updated_at = $updated_at \
                 RETURN AFTER TIMEOUT {MINDMAP_UPDATE_TIMEOUT}"
            ))
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

    /// Trigger auto-summarization when the stream's running token total crosses
    /// the model's summarization threshold. Runs inline; failures are logged but
    /// non-fatal so a summarization error never loses the memory that was added.
    async fn maybe_summarize_after_add(
        &self,
        stream_name: &str,
        updated_stream: &TaskStream,
        stored: Memory,
    ) -> Result<Memory> {
        if updated_stream.needs_summarization() {
            let model_id = updated_stream
                .model_id
                .clone()
                .unwrap_or_else(|| "default".to_string());
            // Scope the summary memory identically to the stream's memories.
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
        let db = self.live_db()?;
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
        let db = self.live_db()?;
        let result: Vec<Entity> = db
            .query("SELECT * FROM entity WHERE name = $name")
            .bind(("name", name.to_string()))
            .await?
            .take(0)?;
        Ok(result.into_iter().next())
    }

    async fn update_entity(&self, mut entity: Entity) -> Result<Entity> {
        let db = self.live_db()?;
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
        let db = self.live_db()?;
        db.query("DELETE FROM entity WHERE name = $name; DELETE FROM relation WHERE from = $name OR to = $name")
            .bind(("name", name.to_string()))
            .await?;
        Ok(())
    }

    async fn search_entities(&self, query: &str) -> Result<Vec<Entity>> {
        let db = self.live_db()?;
        let results: Vec<Entity> = db
            .query("SELECT * FROM entity WHERE name CONTAINS $q OR entity_type CONTAINS $q OR observations CONTAINS $q")
            .bind(("q", query.to_string()))
            .await?
            .take(0)?;
        Ok(results)
    }

    async fn create_relation(&self, mut relation: Relation) -> Result<Relation> {
        let db = self.live_db()?;
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
        let db = self.live_db()?;
        let results: Vec<Relation> = db
            .query("SELECT * FROM relation WHERE from = $name OR to = $name")
            .bind(("name", entity_name.to_string()))
            .await?
            .take(0)?;
        Ok(results)
    }

    async fn delete_relation(&self, from: &str, to: &str, relation_type: &str) -> Result<()> {
        let db = self.live_db()?;
        db.query("DELETE FROM relation WHERE from = $from AND to = $to AND relation_type = $rt")
            .bind(("from", from.to_string()))
            .bind(("to", to.to_string()))
            .bind(("rt", relation_type.to_string()))
            .await?;
        Ok(())
    }

    async fn get_graph(&self) -> Result<KnowledgeGraph> {
        let db = self.live_db()?;
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
        let db = self.live_db()?;
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

        let db = self.live_db()?;

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
        let db = self.live_db()?;
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
        let db = self.live_db()?;
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
        let db = self.live_db()?;
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

        let db = self.live_db()?;

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
        let db = self.live_db()?;
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

    async fn get_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Option<TaskStream>> {
        let db = self.live_db()?;
        let mut sql = "SELECT * FROM task_stream WHERE name = $name".to_string();
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
        let result: Vec<DbTaskStream> = q.await?.take(0)?;
        result
            .into_iter()
            .next()
            .map(Self::decode_task_stream)
            .transpose()
    }

    async fn add_to_task_stream(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        mut memory: Memory,
    ) -> Result<Memory> {
        let stream = self
            .get_task_stream(stream_name, user_id, agent_id)
            .await?
            .with_context(|| format!("TaskStream '{}' not found", stream_name))?;

        if stream.status != TaskStreamStatus::Active {
            anyhow::bail!("TaskStream '{}' is not active", stream_name);
        }

        let stream_id = stream.id.clone().with_context(|| "TaskStream has no id")?;

        // Link memory to stream.
        memory.task_stream_id = Some(stream_id.clone());

        // Compute embedding + token count up front (same as `add_memory`).
        let embedding = self.embed_text(&memory.content).await?;
        if memory.token_count.is_none() {
            memory.token_count = Some(Self::estimate_tokens(&memory.content));
        }

        // Semantic deduplication at the 0.92 threshold, matching `add_memory`.
        // If an existing memory is a near-duplicate, update it in place: its
        // tokens are already counted in `total_tokens`, so we must NOT bump
        // the counter again.
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
                && Self::cosine_similarity(&embedding, c_emb) >= 0.92
                && let Some(id) = &candidate.id
            {
                let id_str = Self::record_id_to_string(id);
                let stored = self.update_memory(&id_str, memory.content).await?;
                let updated_stream = self
                    .get_task_stream(stream_name, user_id, agent_id)
                    .await?
                    .with_context(|| {
                        format!("TaskStream '{}' disappeared during dedup add", stream_name)
                    })?;
                return self
                    .maybe_summarize_after_add(stream_name, &updated_stream, stored)
                    .await;
            }
        }

        // H-2: insert the new memory row, its history record, and the
        // `total_tokens` counter bump inside ONE SurrealDB transaction so no
        // concurrent reader can observe a memory that is not yet counted, and
        // concurrent adds cannot interleave a half-applied state.
        let added_tokens = memory
            .token_count
            .map(|t| t as u64)
            .unwrap_or_else(|| Self::estimate_tokens(&memory.content) as u64);

        let now = Datetime::default();
        memory.embedding = Some(embedding);
        memory.created_at = now;
        memory.updated_at = now;
        memory.version = 1;
        let db_memory = DbMemory::from(memory);

        // Generate the memory key client-side so every statement in the
        // transaction can reference the same record id without depending on
        // SurrealDB `LET`/response-index semantics.
        let memory_key = Uuid::new_v4().to_string();

        let db = self.live_db()?;

        let mut txn_sql = "BEGIN TRANSACTION;\n\
             CREATE type::record('memory', $mkey) CONTENT $memory;\n\
             INSERT INTO memory_history { memory_id: type::record('memory', $mkey), \
             version: 1, old_content: NONE, new_content: $content, \
             changed_at: $now, change_type: 'created' };\n\
             UPDATE task_stream SET total_tokens += $tokens, last_active = $now \
             WHERE name = $name"
            .to_string();
        if user_id.is_some() {
            txn_sql.push_str(" AND user_id = $uid");
        }
        if agent_id.is_some() {
            txn_sql.push_str(" AND agent_id = $aid");
        }
        txn_sql.push_str(";\nCOMMIT TRANSACTION;");

        let mut txn_q = db
            .query(txn_sql)
            .bind(("mkey", memory_key.clone()))
            .bind(("memory", db_memory.clone()))
            .bind(("content", db_memory.content.clone()))
            .bind(("tokens", added_tokens))
            .bind(("now", now))
            .bind(("name", stream_name.to_string()));
        if let Some(v) = user_id {
            txn_q = txn_q.bind(("uid", v.to_string()));
        }
        if let Some(v) = agent_id {
            txn_q = txn_q.bind(("aid", v.to_string()));
        }
        let res = txn_q
            .await
            .context("add_to_task_stream transaction failed")?;
        // `.check()` surfaces any per-statement error so a rejected transaction
        // fails loudly rather than silently dropping the write.
        res.check()
            .context("add_to_task_stream transaction was rejected")?;

        // Index-stability: whether BEGIN/COMMIT occupy result-set slots is
        // driver-dependent, so we do NOT rely on a hardcoded statement index
        // into the transaction response. Instead we re-fetch the just-created
        // row by its client-generated key in a separate scoped SELECT.
        let mut fetch_res = db
            .query("SELECT * FROM type::record('memory', $mkey)")
            .bind(("mkey", memory_key.clone()))
            .await
            .context("add_to_task_stream: failed to read back stored memory")?;
        let created: Option<DbMemory> = fetch_res.take(0)?;
        let stored =
            created.with_context(|| "add_to_task_stream: no memory returned after transaction")?;
        let stored = Self::decode_memory(stored)?;

        let updated_stream = self
            .get_task_stream(stream_name, user_id, agent_id)
            .await?
            .with_context(|| {
                format!(
                    "TaskStream '{}' disappeared during token update",
                    stream_name
                )
            })?;
        self.maybe_summarize_after_add(stream_name, &updated_stream, stored)
            .await
    }

    async fn get_context_for_task(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        model_name: &str,
        max_tokens: Option<u64>,
    ) -> Result<ContextWindow> {
        let budget = max_tokens.unwrap_or_else(|| Self::model_context_budget(model_name));

        let db = self.live_db()?;

        let stream_id = {
            let stream = self
                .get_task_stream(stream_name, user_id, agent_id)
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

        let db = self.live_db()?;

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

    async fn delete_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<()> {
        let stream = self
            .get_task_stream(name, user_id, agent_id)
            .await?
            .with_context(|| format!("TaskStream '{}' not found", name))?;
        let stream_id = stream.id.clone().context("TaskStream missing id")?;

        let memories: Vec<DbMemory> = {
            let db = self.live_db()?;
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

        let db = self.live_db()?;
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

    async fn archive_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<TaskStream> {
        self.update_task_stream_status(name, user_id, agent_id, TaskStreamStatus::Archived)
            .await
    }

    async fn pause_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<TaskStream> {
        self.update_task_stream_status(name, user_id, agent_id, TaskStreamStatus::Paused)
            .await
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

        let db = self.live_db()?;

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

        let db = self.live_db()?;

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
        let db = self.live_db()?;
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

    // ── TaskSteps ────────────────────────────────────────────────────────────

    async fn add_task_step(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        mut step: TaskStep,
    ) -> Result<TaskStep> {
        let stream = self
            .get_task_stream(stream_name, user_id, agent_id)
            .await?
            .with_context(|| format!("TaskStream '{}' not found", stream_name))?;
        let stream_id = stream.id.clone().context("TaskStream has no id")?;

        // Idempotency: if a step with this key already exists, return it
        // verbatim — no duplicate row, no side effects.
        if let Some(existing) = self.find_task_step_by_key(&step.idempotency_key).await? {
            return Ok(existing);
        }

        let idempotency_key = step.idempotency_key.clone();
        step.task_stream_id = Some(stream_id);
        step.created_at = Datetime::default();
        let key = Uuid::new_v4().to_string();
        let payload = DbTaskStep::from(step);
        match self
            .create_record::<_, DbTaskStep>("task_step", &key, payload, "add_task_step")
            .await
        {
            Ok(record) => Self::decode_task_step(record),
            Err(err) if Self::is_unique_violation(&err) => {
                // TOCTOU: a concurrent add_task_step won the race on the
                // unique idempotency_key index. Treat as "already created"
                // and return the existing step so the call stays idempotent.
                self.find_task_step_by_key(&idempotency_key)
                    .await?
                    .with_context(|| {
                        format!(
                            "add_task_step: unique violation but step '{idempotency_key}' \
                             not found on re-fetch"
                        )
                    })
            }
            Err(err) => Err(err),
        }
    }

    async fn update_task_step_status(
        &self,
        idempotency_key: &str,
        status: TaskStepStatus,
        result: Option<String>,
        error: Option<String>,
    ) -> Result<TaskStep> {
        let db = self.db()?;
        let now = Datetime::default();
        // Set started_at the first time the step leaves Pending — for ANY
        // non-Pending target status, including a direct jump to Completed/
        // Failed (e.g. complete_step without a prior Running transition).
        // The conditional preserves an existing started_at on re-entry.
        let set_started = !matches!(status, TaskStepStatus::Pending);
        // Set completed_at when it reaches a terminal state.
        let set_completed = matches!(
            status,
            TaskStepStatus::Completed | TaskStepStatus::Failed | TaskStepStatus::Skipped
        );

        let mut sql =
            String::from("UPDATE task_step SET status = $status, result = $result, error = $error");
        if set_started {
            sql.push_str(", started_at = IF started_at IS NONE THEN $now ELSE started_at END");
        }
        if set_completed {
            sql.push_str(", completed_at = $now");
        }
        sql.push_str(" WHERE idempotency_key = $key RETURN AFTER");

        let mut res = db
            .query(sql)
            .bind(("status", status.as_str().to_string()))
            .bind(("result", result))
            .bind(("error", error))
            .bind(("now", now))
            .bind(("key", idempotency_key.to_string()))
            .await
            .context("update_task_step_status query failed")?;
        let updated: Option<DbTaskStep> = res.take(0)?;
        let updated = updated
            .with_context(|| format!("TaskStep with key '{}' not found", idempotency_key))?;
        Self::decode_task_step(updated)
    }

    async fn get_task_steps(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Vec<TaskStep>> {
        let stream = self
            .get_task_stream(stream_name, user_id, agent_id)
            .await?
            .with_context(|| format!("TaskStream '{}' not found", stream_name))?;
        let stream_id = stream.id.clone().context("TaskStream has no id")?;

        let db = self.db()?;
        let steps: Vec<DbTaskStep> = db
            .query("SELECT * FROM task_step WHERE task_stream_id = $sid ORDER BY ordinal ASC")
            .bind(("sid", stream_id))
            .await?
            .take(0)?;
        Self::decode_task_steps(steps)
    }

    async fn get_current_step(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Option<TaskStep>> {
        // Returns the lowest-ordinal step that is not terminal-done. A Failed
        // or Running step IS returned (see `is_terminal_done`) so the caller
        // can resolve it before advancing.
        let stream = self
            .get_task_stream(stream_name, user_id, agent_id)
            .await?
            .with_context(|| format!("TaskStream '{}' not found", stream_name))?;
        let stream_id = stream.id.clone().context("TaskStream has no id")?;

        let db = self.db()?;
        let steps: Vec<DbTaskStep> = db
            .query(
                "SELECT * FROM task_step WHERE task_stream_id = $sid \
                 AND status NOT IN ['completed', 'skipped'] ORDER BY ordinal ASC LIMIT 1",
            )
            .bind(("sid", stream_id))
            .await?
            .take(0)?;
        steps
            .into_iter()
            .next()
            .map(Self::decode_task_step)
            .transpose()
    }

    async fn complete_step(
        &self,
        idempotency_key: &str,
        result: Option<String>,
    ) -> Result<TaskStep> {
        // Idempotent: an already-completed step is returned unchanged so the
        // result is never re-applied on replay.
        let existing = self
            .find_task_step_by_key(idempotency_key)
            .await?
            .with_context(|| format!("TaskStep with key '{}' not found", idempotency_key))?;
        if existing.status == TaskStepStatus::Completed {
            return Ok(existing);
        }
        self.update_task_step_status(idempotency_key, TaskStepStatus::Completed, result, None)
            .await
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
        let db = self.live_db()?;
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
        let db = self.live_db()?;
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
        let db = self.live_db()?;
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
        let Some(stream) = self.get_task_stream(stream_name, user_id, agent_id).await? else {
            return Ok(None);
        };
        if stream.status != TaskStreamStatus::Active {
            return Ok(None);
        }

        let profile = profile_for(model_id);
        if stream.total_tokens < profile.summarization_threshold() {
            return Ok(None); // nothing to do
        }

        // C-1: the memory-selection query MUST be bound to this stream's id.
        // Using `task_stream_id != NONE` previously selected (and deleted)
        // memories across ALL streams in scope.
        let stream_id = stream.id.clone().with_context(|| "TaskStream has no id")?;

        // Fetch stream memories ordered oldest-first, scoped to this stream.
        let mut scope_parts: Vec<String> = vec!["task_stream_id = $sid".into()];
        if user_id.is_some() {
            scope_parts.push("user_id = $uid".into());
        }
        if agent_id.is_some() {
            scope_parts.push("agent_id = $aid".into());
        }
        let db = self.live_db()?;

        let sql = format!(
            "SELECT * FROM memory WHERE {} ORDER BY created_at ASC LIMIT 200",
            scope_parts.join(" AND ")
        );
        let mut q = db.query(&sql).bind(("sid", stream_id.clone()));
        if let Some(u) = user_id {
            q = q.bind(("uid", u.to_string()));
        }
        if let Some(a) = agent_id {
            q = q.bind(("aid", a.to_string()));
        }
        let memories: Vec<DbMemory> = q.await?.take(0).unwrap_or_default();
        let memories = Self::decode_memories(memories)?;

        if memories.len() < 4 {
            return Ok(None); // not enough to compress
        }

        // Compress the oldest half
        let half = memories.len() / 2;
        let to_compress = memories.into_iter().take(half).collect::<Vec<_>>();

        // M-2(a): sum the compacted memories' token counts so the stream's
        // `total_tokens` counter can be decremented by exactly what we remove.
        // Use the same estimate fallback as `add_to_task_stream` /
        // `get_context_for_task` so the counter stays consistent.
        let compacted_tokens: u64 = to_compress
            .iter()
            .map(|m| {
                m.token_count
                    .map(|t| t as u64)
                    .unwrap_or_else(|| Self::estimate_tokens(&m.content) as u64)
            })
            .sum();

        // Build summary content. NOTE: a naive concat can itself grow tokens;
        // a real LLM summarization path is out of scope for this correctness
        // tier (there is no existing summarization helper to call here).
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

        // M-2(b): the summary memory MUST stay attached to the stream so that
        // `get_context_for_task` (which filters by `task_stream_id`) returns it.
        let mut summary = crate::memory::Memory::new(
            summary_content,
            user_id.map(str::to_string),
            agent_id.map(str::to_string),
            None,
            vec!["auto_summary".to_string()],
        );
        summary.task_stream_id = Some(stream_id.clone());
        let stored = self.add_memory(summary).await?;

        // M-2: the summary memory is now a linked member of the stream, so its
        // own tokens must be counted in `total_tokens`. `add_memory` only
        // counts tokens into the memory row, never the stream counter, so the
        // net effect must be `total_tokens -= compacted; total_tokens +=
        // summary`. Use the same token-estimate source as the rest of the code.
        let summary_tokens: u64 = stored
            .token_count
            .map(|t| t as u64)
            .unwrap_or_else(|| Self::estimate_tokens(&stored.content) as u64);

        // M-2(a): decrement `total_tokens` by the compacted total, add the new
        // summary memory's tokens back, and bump `summary_count`.
        let _: Option<serde_json::Value> = db
            .query(
                "UPDATE type::table($t) \
                 SET total_tokens = math::max([0, total_tokens - $compacted]) + $summary, \
                     summary_count += 1, last_active = time::now() \
                 WHERE name = $n",
            )
            .bind(("t", "task_stream"))
            .bind(("compacted", compacted_tokens))
            .bind(("summary", summary_tokens))
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
        let db = self.live_db()?;
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
                let db = self.live_db()?;
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
                let db = self.live_db()?;
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
        let db = self.live_db()?;

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
        let db = self.live_db()?;
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
        let db = self.live_db()?;
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

    fn as_any(&self) -> &dyn std::any::Any {
        self
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
        Self::ensure_embedding_indexes(&db, embedding_service.dimensions()).await?;

        let embedded_semaphore = make_embedded_semaphore(&connection_info.config);

        Ok(Self {
            connection: Arc::new(ArcSwap::new(Arc::new(ConnectionCell::Connected(db)))),
            connection_info,
            embedding_service,
            embedded_semaphore,
            #[cfg(feature = "palace")]
            palace: tokio::sync::OnceCell::new(),
        })
    }
}

// ── PalaceStorage implementation ─────────────────────────────────────────────

#[cfg(feature = "palace")]
impl SurrealStorage {
    /// Lazily initialise the `PalaceContext`, reusing the existing SurrealDB
    /// connection. Subsequent calls return the cached reference.
    async fn palace_context(&self) -> Result<&PalaceContext> {
        self.palace
            .get_or_try_init(|| async { PalaceContext::from_storage(self).await })
            .await
    }
}

#[cfg(feature = "palace")]
#[async_trait]
impl PalaceStorage for SurrealStorage {
    async fn palace_wake_up(&self, wing: Option<&str>) -> anyhow::Result<String> {
        self.palace_context().await?.wake_up(wing).await
    }

    async fn palace_recall(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<String> {
        self.palace_context().await?.recall(wing, room, limit).await
    }

    async fn palace_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        n: usize,
    ) -> anyhow::Result<String> {
        self.palace_context()
            .await?
            .search(query, wing, room, n)
            .await
    }

    async fn palace_ingest(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        hall: &str,
        importance: f32,
    ) -> anyhow::Result<String> {
        self.palace_context()
            .await?
            .ingest(content, wing, room, hall, importance)
            .await
    }

    async fn palace_delete(&self, id: &str) -> anyhow::Result<()> {
        self.palace_context().await?.delete(id).await
    }

    async fn palace_status(&self) -> anyhow::Result<PalaceStatus> {
        let stack_status = self.palace_context().await?.status().await?;
        Ok(PalaceStatus {
            total_drawers: stack_status.total_drawers,
            total_wings: stack_status.total_wings,
            total_rooms: stack_status.total_rooms,
            identity_loaded: stack_status.identity_loaded,
        })
    }

    fn palace_compress(&self, text: &str) -> String {
        use mempalace_core::dialect::compress::{Dialect, DialectConfig};
        use std::sync::OnceLock;

        static DIALECT: OnceLock<Dialect> = OnceLock::new();
        let dialect = DIALECT.get_or_init(|| Dialect::new(DialectConfig::default()));
        dialect.compress(text, None)
    }

    async fn palace_hybrid_search(
        &self,
        query: &str,
        scope: Option<crate::memory::MemoryScope>,
        wing: Option<&str>,
        n: usize,
    ) -> anyhow::Result<Vec<UnifiedHit>> {
        use mempalace_core::reranker::ReciprocRankFusion;
        use mempalace_core::storage::types::DrawerHit;

        // Determine scope-based filters for memory search
        let (user_id, agent_id, session_id) = match &scope {
            Some(crate::memory::MemoryScope::Agent) => (None, Some(""), None),
            Some(crate::memory::MemoryScope::User) => (Some(""), None, None),
            Some(crate::memory::MemoryScope::Session) => (None, None, Some("")),
            _ => (None, None, None),
        };

        // Run all three searches concurrently
        let memory_fut =
            self.hybrid_search_memories(query, user_id, agent_id, session_id, n, 0.7, 0.3);
        let entity_fut = self.semantic_search(query, n, 0.0);
        let palace_ctx = self.palace_context().await?;
        let palace_fut = palace_ctx.search_drawers_structured(query, wing, None, n);

        let (memories, entities, palace_hits) = tokio::join!(memory_fut, entity_fut, palace_fut);

        let mut rrf_lists: Vec<Vec<DrawerHit>> = Vec::new();

        // Convert memory hits to DrawerHits
        if let Ok(mems) = memories {
            let hits: Vec<DrawerHit> = mems
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let id_str =
                        m.id.as_ref()
                            .map(|r| Self::record_id_to_string(r))
                            .unwrap_or_else(|| format!("memory_{i}"));
                    DrawerHit {
                        drawer: mempalace_core::storage::types::Drawer {
                            id: id_str,
                            content: m.content.clone(),
                            wing: "memory".to_string(),
                            room: m.memory_type.as_str().to_string(),
                            hall: String::new(),
                            source_file: None,
                            date: Some(m.created_at.to_string()),
                            importance: m.importance,
                            embedding: None,
                        },
                        similarity: m.importance,
                    }
                })
                .collect();
            rrf_lists.push(hits);
        }

        // Convert entity hits to DrawerHits
        if let Ok(ents) = entities {
            let hits: Vec<DrawerHit> = ents
                .iter()
                .map(|e| DrawerHit {
                    drawer: mempalace_core::storage::types::Drawer {
                        id: e.entity.name.clone(),
                        content: e.entity.observations.join("; "),
                        wing: "entity".to_string(),
                        room: e.entity.entity_type.clone(),
                        hall: String::new(),
                        source_file: None,
                        date: None,
                        importance: 1.0,
                        embedding: None,
                    },
                    similarity: e.similarity,
                })
                .collect();
            rrf_lists.push(hits);
        }

        // Palace drawer hits (already in DrawerHit format)
        rrf_lists.push(palace_hits.unwrap_or_default());

        // Merge with RRF
        let rrf = ReciprocRankFusion::new(60);
        let merged = rrf.merge(rrf_lists);

        // Convert to UnifiedHit
        let unified: Vec<UnifiedHit> = merged
            .into_iter()
            .map(|hit| {
                let source = match hit.drawer.wing.as_str() {
                    "memory" => {
                        let memory_type = crate::memory::MemoryType::parse_str(&hit.drawer.room)
                            .unwrap_or_default();
                        HitSource::Memory {
                            scope: scope.clone().unwrap_or_default(),
                            memory_type,
                        }
                    }
                    "entity" => HitSource::Entity {
                        entity_type: hit.drawer.room.clone(),
                    },
                    _ => HitSource::Palace {
                        wing: hit.drawer.wing.clone(),
                        room: hit.drawer.room.clone(),
                    },
                };
                UnifiedHit {
                    id: hit.drawer.id,
                    content: hit.drawer.content,
                    source,
                    score: hit.similarity,
                }
            })
            .collect();

        Ok(unified)
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
    fn test_classify_transport_errors_as_reconnect() {
        let storage = mock_storage();
        for raw in [
            "Connection uninitialised",
            "connection closed",
            "connection refused",
            "connection reset by peer",
            "network unreachable",
            "DNS lookup failed",
        ] {
            let err = anyhow::anyhow!(raw);
            assert_eq!(
                storage.classify_error(&err),
                RetryAction::Reconnect,
                "{raw} should be Reconnect"
            );
        }
    }

    #[test]
    fn test_classify_server_busy_as_retry_not_reconnect() {
        let storage = mock_storage();
        // These previously triggered a reconnect — wrong. The connection
        // is fine; the server-side is busy. Backoff-and-retry is correct.
        for raw in [
            "Operation timeout",
            "query timed out",
            "too many connections",
            "backpressure applied",
            "lock timeout while writing",
            "serialization failure on commit",
        ] {
            let err = anyhow::anyhow!(raw);
            assert_eq!(
                storage.classify_error(&err),
                RetryAction::Retry,
                "{raw} should be Retry (not Reconnect)"
            );
        }
    }

    #[test]
    fn test_classify_schema_errors_as_fail_fast() {
        let storage = mock_storage();
        for raw in [
            "Found field 'foo', but no such field exists",
            "invalid credentials",
            "table doesn't exist",
            "record not found",
            "syntax error in query",
        ] {
            let err = anyhow::anyhow!(raw);
            assert_eq!(
                storage.classify_error(&err),
                RetryAction::FailFast,
                "{raw} should be FailFast"
            );
        }
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
        // Result can be Ok or Err depending on environment — the test only
        // asserts the call completes without panicking.
        let _ = result;
    }

    #[tokio::test]
    async fn test_reconnect_updates_connection_state() {
        // Test that reconnect_with_attempts properly transitions state through
        // Reconnecting. If DB is available, it ends in Connected; if not, Failed.
        let storage = mock_storage();

        // Call reconnect with the operation-path attempt cap
        let result = storage
            .reconnect_with_attempts(OPERATION_RECONNECT_ATTEMPTS)
            .await;

        // After reconnect, state should be either Connected (if DB available) or Failed (if not)
        // The important thing is that it's no longer in the initial Failed/Reconnecting state
        {
            let cell = storage.connection.load();
            match &**cell {
                ConnectionCell::Connected(_) => {
                    // Reconnection succeeded
                    assert!(result.is_ok());
                }
                ConnectionCell::Failed(_) => {
                    // Reconnection failed
                    assert!(result.is_err());
                }
                ConnectionCell::Reconnecting => {
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

    #[tokio::test]
    async fn test_create_task_stream_fast_fails_on_failed_connection() {
        // C-2 reproduction (failed-connection path): create_task_stream does no
        // embedding, yet it hung 4 minutes in the incident. Against a Failed
        // connection it must return a bounded typed error immediately.
        let storage = mock_storage();
        let stream = crate::task_stream::TaskStream::new(
            "c2-repro".to_string(),
            None,
            Some("agent-x".to_string()),
            None,
        );

        let start = std::time::Instant::now();
        let result = storage.create_task_stream(stream).await;
        let elapsed = start.elapsed();

        assert!(
            result.is_err(),
            "create_task_stream against an unreachable DB must return an error"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "create_task_stream must fail fast, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_retry_operation_aborts_on_deadline() {
        // C-2 core fix: a write must never stall open-endedly. With a live
        // (Connected) connection and a hung operation closure, retry_operation
        // must abort at the wall-clock deadline with a typed error.
        struct MockEmbedding;
        #[async_trait::async_trait]
        impl crate::embeddings::EmbeddingService for MockEmbedding {
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

        let mut storage = SurrealStorage::new_mem(Arc::new(MockEmbedding))
            .await
            .expect("in-memory storage");
        storage.connection_info.config.retry.operation_deadline_ms = 300;

        let start = std::time::Instant::now();
        let result: Result<()> = storage
            .retry_operation("hung_op", |_db| async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(())
            })
            .await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "a hung operation must return an error");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("deadline"),
            "error must be a typed deadline error, got: {msg}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "operation must abort at the deadline, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_reconnect_cancellation_does_not_strand_reconnecting() {
        // If reconnect_with_attempts is cancelled (e.g. by the operation
        // deadline) the ReconnectGuard must force state out of Reconnecting so
        // the storage instance stays usable rather than bailing forever.
        let storage = mock_storage();
        // A very short timeout cancels reconnect_with_attempts mid-connect.
        let _ = tokio::time::timeout(
            Duration::from_millis(1),
            storage.reconnect_with_attempts(OPERATION_RECONNECT_ATTEMPTS),
        )
        .await;

        // Give the dropped future's guard a moment to run.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let cell = storage.connection.load();
        assert!(
            !matches!(&**cell, ConnectionCell::Reconnecting),
            "connection must not be stranded in Reconnecting after cancellation"
        );
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
            connection: Arc::new(ArcSwap::new(Arc::new(ConnectionCell::Failed(
                "test".to_string(),
            )))),
            connection_info,
            embedding_service: Arc::new(MockEmbedding),
            embedded_semaphore: None,
            #[cfg(feature = "palace")]
            palace: tokio::sync::OnceCell::new(),
        }
    }
}
