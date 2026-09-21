//! Durable, receipt-driven operation ingestion.
//!
//! The operation ledger is the acknowledgement boundary.  A caller-provided
//! id is persisted before any model inference starts, and every subsequent
//! action is recoverable from explicit database state.  Wall-clock duration is
//! never used to infer whether an operation succeeded.

use std::{
    collections::{HashSet, VecDeque},
    convert::Infallible,
    future::{Future, IntoFuture},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use arc_swap::ArcSwapOption;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use chrono::Utc;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use surreal_memory::{
    EmbeddingService, MemoryStorage, SurrealStorage, TaskStep, TaskStream,
    embeddings::{EmbeddingPlanPart, ExecutorEvent, ExecutorEventKind, ExecutorSnapshot},
};
use surrealdb::types::{Datetime, RecordId};
use surrealdb::{Surreal, engine::any::Any};
use surrealdb_types::SurrealValue;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;

use crate::contracts::AddMemoryRequest;

use crate::api::{ApiError, ApiFailure, AppState, bad_request, not_found};

const OPERATION_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Accepted,
    Validated,
    Blocked,
    Planned,
    Processing,
    Indexed,
    Committed,
    Rejected,
}

impl OperationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Validated => "validated",
            Self::Blocked => "blocked",
            Self::Planned => "planned",
            Self::Processing => "processing",
            Self::Indexed => "indexed",
            Self::Committed => "committed",
            Self::Rejected => "rejected",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Committed | Self::Rejected)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRequest {
    pub operation_id: String,
    pub schema_version: u32,
    pub kind: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub payload_hash: String,
    pub payload: Value,
}

impl OperationRequest {
    fn validate(&self) -> Result<()> {
        if self.operation_id.trim().is_empty() {
            anyhow::bail!("operation_id cannot be empty");
        }
        if self.schema_version != OPERATION_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported operation schema {}; expected {}",
                self.schema_version,
                OPERATION_SCHEMA_VERSION
            );
        }
        if !matches!(
            self.kind.as_str(),
            "add_memory" | "create_task_stream" | "add_task_step" | "complete_step"
        ) {
            anyhow::bail!("unknown operation kind '{}'", self.kind);
        }
        if self.dependencies.iter().any(|id| id.trim().is_empty()) {
            anyhow::bail!("dependencies cannot contain an empty operation id");
        }
        let actual = payload_hash(&self.payload)?;
        if actual != self.payload_hash {
            anyhow::bail!(
                "payload_hash mismatch: request supplied {}, canonical payload is {}",
                self.payload_hash,
                actual
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationReceipt {
    pub operation_id: String,
    pub schema_version: u32,
    pub kind: String,
    pub payload_hash: String,
    pub dependencies: Vec<String>,
    pub state: OperationState,
    pub blocked_by: Vec<String>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub executor_generation: u64,
    pub executor_progress_seq: u64,
    pub executor_exit_count: u64,
    pub executor_last_exit: Option<String>,
    pub executor_error: Option<String>,
    pub progress_seq: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEvent {
    pub operation_id: String,
    pub sequence: u64,
    pub from_state: Option<String>,
    pub to_state: String,
    pub detail: Option<Value>,
    pub executor_generation: u64,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbOperation {
    #[serde(default)]
    id: Option<RecordId>,
    operation_id: String,
    schema_version: u32,
    kind: String,
    dependencies: Vec<String>,
    payload_hash: String,
    payload: Value,
    state: String,
    blocked_by: Vec<String>,
    result: Option<Value>,
    error: Option<String>,
    executor_generation: u64,
    #[serde(default)]
    executor_progress_seq: u64,
    #[serde(default)]
    executor_exit_count: u64,
    #[serde(default)]
    executor_last_exit: Option<String>,
    #[serde(default)]
    executor_error: Option<String>,
    progress_seq: u64,
    created_at: Datetime,
    updated_at: Datetime,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbOperationId {
    operation_id: String,
}

const LIST_OPERATION_IDS_BY_STATE_QUERY: &str =
    "SELECT operation_id FROM memory_operation WHERE state = $state ORDER BY operation_id ASC";
const NONTERMINAL_OPERATION_STATES: [&str; 4] = ["accepted", "validated", "blocked", "processing"];

#[derive(Debug, Deserialize, SurrealValue)]
struct DbOperationReceipt {
    operation_id: String,
    schema_version: u32,
    kind: String,
    dependencies: Vec<String>,
    payload_hash: String,
    state: String,
    blocked_by: Vec<String>,
    result: Option<Value>,
    error: Option<String>,
    executor_generation: u64,
    #[serde(default)]
    executor_progress_seq: u64,
    #[serde(default)]
    executor_exit_count: u64,
    #[serde(default)]
    executor_last_exit: Option<String>,
    #[serde(default)]
    executor_error: Option<String>,
    progress_seq: u64,
    created_at: Datetime,
    updated_at: Datetime,
}

const GET_OPERATION_RECEIPT_QUERY: &str = "SELECT operation_id, schema_version, kind, dependencies, payload_hash, state, blocked_by, result, error, executor_generation, executor_progress_seq, executor_exit_count, executor_last_exit, executor_error, progress_seq, created_at, updated_at FROM memory_operation WHERE operation_id = $id LIMIT 1";

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbOperationEvent {
    #[serde(default)]
    id: Option<RecordId>,
    operation_id: String,
    sequence: u64,
    from_state: Option<String>,
    to_state: String,
    detail: Option<Value>,
    executor_generation: u64,
    occurred_at: Datetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbOperationPart {
    #[serde(default)]
    id: Option<RecordId>,
    operation_id: String,
    part_index: u64,
    token_start: u64,
    token_end: u64,
    token_count: u64,
    token_hash: String,
    content: String,
    state: String,
    embedding: Option<Vec<f32>>,
    updated_at: Datetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbExecutorEvent {
    #[serde(default)]
    id: Option<RecordId>,
    operation_id: String,
    generation: u64,
    progress_seq: u64,
    kind: String,
    message: Option<String>,
    occurred_at: Datetime,
}

impl TryFrom<DbOperation> for OperationReceipt {
    type Error = anyhow::Error;

    fn try_from(value: DbOperation) -> Result<Self> {
        Ok(Self {
            operation_id: value.operation_id,
            schema_version: value.schema_version,
            kind: value.kind,
            payload_hash: value.payload_hash,
            dependencies: value.dependencies,
            state: parse_state(&value.state)?,
            blocked_by: value.blocked_by,
            result: value.result,
            error: value.error,
            executor_generation: value.executor_generation,
            executor_progress_seq: value.executor_progress_seq,
            executor_exit_count: value.executor_exit_count,
            executor_last_exit: value.executor_last_exit,
            executor_error: value.executor_error,
            progress_seq: value.progress_seq,
            created_at: value.created_at.to_string(),
            updated_at: value.updated_at.to_string(),
        })
    }
}

impl TryFrom<DbOperationReceipt> for OperationReceipt {
    type Error = anyhow::Error;

    fn try_from(value: DbOperationReceipt) -> Result<Self> {
        Ok(Self {
            operation_id: value.operation_id,
            schema_version: value.schema_version,
            kind: value.kind,
            payload_hash: value.payload_hash,
            dependencies: value.dependencies,
            state: parse_state(&value.state)?,
            blocked_by: value.blocked_by,
            result: value.result,
            error: value.error,
            executor_generation: value.executor_generation,
            executor_progress_seq: value.executor_progress_seq,
            executor_exit_count: value.executor_exit_count,
            executor_last_exit: value.executor_last_exit,
            executor_error: value.executor_error,
            progress_seq: value.progress_seq,
            created_at: value.created_at.to_string(),
            updated_at: value.updated_at.to_string(),
        })
    }
}

impl From<DbOperationEvent> for OperationEvent {
    fn from(value: DbOperationEvent) -> Self {
        Self {
            operation_id: value.operation_id,
            sequence: value.sequence,
            from_state: value.from_state,
            to_state: value.to_state,
            detail: value.detail,
            executor_generation: value.executor_generation,
            occurred_at: value.occurred_at.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct OperationService {
    storage: Arc<dyn MemoryStorage>,
    embedding_service: Arc<dyn EmbeddingService>,
    query_timeout: Duration,
    ledger_connector: LedgerConnector,
    ledger_connection: Arc<ArcSwapOption<LedgerConnection>>,
    connection_replacement: Arc<tokio::sync::Mutex<()>>,
    wake_tx: mpsc::Sender<String>,
    events_tx: broadcast::Sender<OperationEvent>,
    #[cfg(test)]
    process_override: Option<ProcessOverride>,
}

struct LedgerConnection {
    generation: u64,
    db: Surreal<Any>,
}

type LedgerConnectFuture = Pin<Box<dyn Future<Output = Result<Surreal<Any>>> + Send>>;
type LedgerConnector = Arc<dyn Fn() -> LedgerConnectFuture + Send + Sync>;
#[cfg(test)]
type ProcessOverrideFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
#[cfg(test)]
type ProcessOverride = Arc<dyn Fn(String) -> ProcessOverrideFuture + Send + Sync>;

#[derive(Debug, thiserror::Error)]
#[error("operation database {stage} timed out after {timeout_ms}ms")]
struct LedgerDeadlineExceeded {
    stage: &'static str,
    timeout_ms: u128,
    recovered: bool,
}

fn is_recovered_ledger_deadline(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<LedgerDeadlineExceeded>()
            .is_some_and(|deadline| deadline.recovered)
    })
}

async fn retry_recovered_ledger_once<T, F, Fut>(mut operation: F) -> (Result<T>, bool)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match operation().await {
        Err(error) if is_recovered_ledger_deadline(&error) => (operation().await, true),
        result => (result, false),
    }
}

#[derive(Debug)]
pub enum SubmitError {
    Invalid(anyhow::Error),
    Conflict(Box<OperationReceipt>),
    Storage(anyhow::Error),
}

impl OperationService {
    pub fn start(
        storage: Arc<dyn MemoryStorage>,
        embedding_service: Arc<dyn EmbeddingService>,
    ) -> Self {
        Self::start_with_query_timeout(storage, embedding_service, Duration::from_secs(10))
    }

    pub fn start_with_query_timeout(
        storage: Arc<dyn MemoryStorage>,
        embedding_service: Arc<dyn EmbeddingService>,
        query_timeout: Duration,
    ) -> Self {
        Self::start_with_capacities(storage, embedding_service, query_timeout, 256, 1024)
    }

    fn start_with_capacities(
        storage: Arc<dyn MemoryStorage>,
        embedding_service: Arc<dyn EmbeddingService>,
        query_timeout: Duration,
        wake_capacity: usize,
        event_capacity: usize,
    ) -> Self {
        let (wake_tx, wake_rx) = mpsc::channel(wake_capacity);
        let (events_tx, _) = broadcast::channel(event_capacity);
        let connector_storage = Arc::clone(&storage);
        let ledger_connector: LedgerConnector = Arc::new(move || {
            let storage = Arc::clone(&connector_storage);
            Box::pin(async move {
                let surreal = storage
                    .as_any()
                    .downcast_ref::<SurrealStorage>()
                    .context("durable operations require SurrealStorage")?;
                surreal.operation_ledger_connection().await
            })
        });
        let service = Self {
            storage,
            embedding_service,
            query_timeout,
            ledger_connector,
            ledger_connection: Arc::new(ArcSwapOption::empty()),
            connection_replacement: Arc::new(tokio::sync::Mutex::new(())),
            wake_tx,
            events_tx,
            #[cfg(test)]
            process_override: None,
        };
        if let Some(executor_events) = service.embedding_service.subscribe_executor_events() {
            let journal = service.clone();
            tokio::spawn(async move { journal.record_executor_events(executor_events).await });
        }
        let coordinator = service.clone();
        tokio::spawn(async move { coordinator.run(wake_rx).await });
        service
    }

    fn surreal(&self) -> Result<&SurrealStorage> {
        self.storage
            .as_any()
            .downcast_ref::<SurrealStorage>()
            .context("durable operations require SurrealStorage")
    }

    async fn ledger_connection(&self) -> Result<Arc<LedgerConnection>> {
        if let Some(connection) = self.ledger_connection.load_full() {
            return Ok(connection);
        }
        let _replacement = self.connection_replacement.lock().await;
        if let Some(connection) = self.ledger_connection.load_full() {
            return Ok(connection);
        }
        let db = (self.ledger_connector)().await?;
        let connection = Arc::new(LedgerConnection { generation: 0, db });
        self.ledger_connection.store(Some(Arc::clone(&connection)));
        Ok(connection)
    }

    async fn replace_ledger_connection(&self, stale_generation: u64) -> Result<()> {
        let _replacement = self.connection_replacement.lock().await;
        if self
            .ledger_connection
            .load_full()
            .is_some_and(|connection| connection.generation != stale_generation)
        {
            return Ok(());
        }
        let db = (self.ledger_connector)().await?;
        self.ledger_connection
            .store(Some(Arc::new(LedgerConnection {
                generation: stale_generation.saturating_add(1),
                db,
            })));
        Ok(())
    }

    async fn await_database<T, E, F>(
        &self,
        stage: &'static str,
        generation: u64,
        future: F,
    ) -> Result<T>
    where
        F: IntoFuture<Output = std::result::Result<T, E>>,
        anyhow::Error: From<E>,
    {
        match tokio::time::timeout(self.query_timeout, future.into_future()).await {
            Ok(result) => result.map_err(anyhow::Error::from),
            Err(_) => {
                let replacement_timeout = self.query_timeout.max(Duration::from_secs(1));
                let replacement = tokio::time::timeout(
                    replacement_timeout,
                    self.replace_ledger_connection(generation),
                )
                .await;
                let recovered = match replacement {
                    Ok(Ok(())) => true,
                    Ok(Err(error)) => {
                        tracing::error!(%error, stage, "operation database connection replacement failed");
                        false
                    }
                    Err(_) => {
                        tracing::error!(
                            stage,
                            timeout_ms = replacement_timeout.as_millis(),
                            "operation database connection replacement timed out"
                        );
                        false
                    }
                };
                Err(LedgerDeadlineExceeded {
                    stage,
                    timeout_ms: self.query_timeout.as_millis(),
                    recovered,
                }
                .into())
            }
        }
    }

    pub async fn submit(
        &self,
        request: OperationRequest,
    ) -> Result<(OperationReceipt, bool), SubmitError> {
        request.validate().map_err(SubmitError::Invalid)?;
        if let Some(existing) = self
            .get(&request.operation_id)
            .await
            .map_err(SubmitError::Storage)?
        {
            if existing.payload_hash != request.payload_hash {
                return Err(SubmitError::Conflict(Box::new(existing)));
            }
            if !existing.state.is_terminal() {
                let _ = self.wake_tx.send(existing.operation_id.clone()).await;
            }
            return Ok((existing, false));
        }

        let now = Datetime::default();
        let request_payload_hash = request.payload_hash.clone();
        let db_record = DbOperation {
            id: None,
            operation_id: request.operation_id.clone(),
            schema_version: request.schema_version,
            kind: request.kind,
            dependencies: request.dependencies,
            payload_hash: request_payload_hash.clone(),
            payload: request.payload,
            state: OperationState::Accepted.as_str().to_owned(),
            blocked_by: Vec::new(),
            result: None,
            error: None,
            executor_generation: 0,
            executor_progress_seq: 0,
            executor_exit_count: 0,
            executor_last_exit: None,
            executor_error: None,
            progress_seq: 1,
            created_at: now,
            updated_at: now,
        };
        let event = DbOperationEvent {
            id: None,
            operation_id: request.operation_id.clone(),
            sequence: 1,
            from_state: None,
            to_state: OperationState::Accepted.as_str().to_owned(),
            detail: Some(json!({"receipt":"durably accepted"})),
            executor_generation: 0,
            occurred_at: now,
        };
        let key = record_key(&request.operation_id);
        let event_key = format!("{key}-0000000000000001");
        let connection = self
            .ledger_connection()
            .await
            .map_err(SubmitError::Storage)?;
        let db = connection.db.clone();
        let response = self
            .await_database(
                "submit",
                connection.generation,
                db.query(
                    "BEGIN TRANSACTION;\n\
                 CREATE type::record('memory_operation', $key) CONTENT $operation;\n\
                 CREATE type::record('memory_operation_event', $event_key) CONTENT $event;\n\
                 COMMIT TRANSACTION;",
                )
                .bind(("key", key))
                .bind(("operation", db_record))
                .bind(("event_key", event_key))
                .bind(("event", event)),
            )
            .await;

        if let Err(error) = response.and_then(|response| response.check().map_err(Into::into)) {
            // A concurrent submit may have won the unique-index race. Re-read
            // the authoritative row and apply the same hash rule.
            if let Some(existing) = self
                .get(&request.operation_id)
                .await
                .map_err(SubmitError::Storage)?
            {
                if existing.payload_hash == request_payload_hash {
                    return Ok((existing, false));
                }
                return Err(SubmitError::Conflict(Box::new(existing)));
            }
            return Err(SubmitError::Storage(error));
        }

        let receipt = self
            .get(&request.operation_id)
            .await
            .map_err(SubmitError::Storage)?
            .context("accepted operation disappeared")
            .map_err(SubmitError::Storage)?;
        let accepted_event = OperationEvent {
            operation_id: receipt.operation_id.clone(),
            sequence: 1,
            from_state: None,
            to_state: OperationState::Accepted.as_str().to_owned(),
            detail: Some(json!({"receipt":"durably accepted"})),
            executor_generation: 0,
            occurred_at: Utc::now().to_rfc3339(),
        };
        let _ = self.events_tx.send(accepted_event);
        let _ = self.wake_tx.send(receipt.operation_id.clone()).await;
        Ok((receipt, true))
    }

    pub async fn get(&self, operation_id: &str) -> Result<Option<OperationReceipt>> {
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        let mut rows: Vec<DbOperationReceipt> = self
            .await_database(
                "receipt lookup",
                connection.generation,
                db.query(GET_OPERATION_RECEIPT_QUERY)
                    .bind(("id", operation_id.to_owned())),
            )
            .await?
            .check()?
            .take(0)?;
        rows.pop().map(TryInto::try_into).transpose()
    }

    async fn get_db(&self, operation_id: &str) -> Result<Option<DbOperation>> {
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        self.await_database(
            "operation lookup",
            connection.generation,
            db.select(("memory_operation", record_key(operation_id))),
        )
        .await
    }

    async fn list_nonterminal_ids(&self) -> Result<Vec<String>> {
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        let mut operation_ids = Vec::new();
        for state in NONTERMINAL_OPERATION_STATES {
            let rows: Vec<DbOperationId> = self
                .await_database(
                    "reconciliation discovery",
                    connection.generation,
                    db.query(LIST_OPERATION_IDS_BY_STATE_QUERY)
                        .bind(("state", state)),
                )
                .await?
                .check()?
                .take(0)?;
            operation_ids.extend(rows.into_iter().map(|row| row.operation_id));
        }
        operation_ids.sort_unstable();
        operation_ids.dedup();
        Ok(operation_ids)
    }

    async fn events_after(&self, operation_id: &str, sequence: u64) -> Result<Vec<OperationEvent>> {
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        let rows: Vec<DbOperationEvent> = self
            .await_database(
                "event history lookup",
                connection.generation,
                db.query(
                "SELECT * FROM memory_operation_event WHERE operation_id = $id AND sequence > $sequence ORDER BY sequence ASC",
            )
            .bind(("id", operation_id.to_owned()))
            .bind(("sequence", sequence)),
            )
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn transition(
        &self,
        operation_id: &str,
        to: OperationState,
        blocked_by: Vec<String>,
        result: Option<Value>,
        error: Option<String>,
        detail: Option<Value>,
    ) -> Result<OperationReceipt> {
        let current = self
            .get_db(operation_id)
            .await?
            .with_context(|| format!("operation '{operation_id}' not found"))?;
        let from = current.state.clone();
        let sequence = current.progress_seq + 1;
        let now = Datetime::default();
        let event = DbOperationEvent {
            id: None,
            operation_id: operation_id.to_owned(),
            sequence,
            from_state: Some(from.clone()),
            to_state: to.as_str().to_owned(),
            detail: detail.clone(),
            executor_generation: current.executor_generation,
            occurred_at: now,
        };
        let event_key = format!("{}-{sequence:016}", record_key(operation_id));
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        self.await_database(
            "state transition",
            connection.generation,
            db.query(
            "BEGIN TRANSACTION;\n\
             UPDATE memory_operation SET state = $state, blocked_by = $blocked_by, result = $result, error = $error, progress_seq = $sequence, updated_at = $now WHERE operation_id = $id;\n\
             CREATE type::record('memory_operation_event', $event_key) CONTENT $event;\n\
             COMMIT TRANSACTION;",
        )
        .bind(("state", to.as_str().to_owned()))
        .bind(("blocked_by", blocked_by))
        .bind(("result", result))
        .bind(("error", error))
        .bind(("sequence", sequence))
        .bind(("now", now))
        .bind(("id", operation_id.to_owned()))
        .bind(("event_key", event_key))
        .bind(("event", event)),
        )
        .await?
        .check()?;
        let published = OperationEvent {
            operation_id: operation_id.to_owned(),
            sequence,
            from_state: Some(from),
            to_state: to.as_str().to_owned(),
            detail,
            executor_generation: current.executor_generation,
            occurred_at: Utc::now().to_rfc3339(),
        };
        let _ = self.events_tx.send(published);
        self.get(operation_id)
            .await?
            .context("transitioned operation disappeared")
    }

    async fn dependency_blockers(&self, operation: &DbOperation) -> Result<Vec<String>> {
        let mut blockers = Vec::new();
        for dependency in &operation.dependencies {
            match self.get(dependency).await? {
                Some(receipt) if receipt.state == OperationState::Committed => {}
                _ => blockers.push(dependency.clone()),
            }
        }
        blockers.sort();
        blockers.dedup();
        Ok(blockers)
    }

    async fn operation_parts(&self, operation_id: &str) -> Result<Vec<DbOperationPart>> {
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        let parts: Vec<DbOperationPart> = self
            .await_database(
                "part lookup",
                connection.generation,
                db.query(
                "SELECT * FROM memory_operation_part WHERE operation_id = $id ORDER BY part_index ASC",
            )
            .bind(("id", operation_id.to_owned())),
            )
            .await?
            .check()?
            .take(0)?;
        Ok(parts)
    }

    async fn persist_plan(&self, operation_id: &str, plan: &[EmbeddingPlanPart]) -> Result<()> {
        let existing = self.operation_parts(operation_id).await?;
        if !existing.is_empty() {
            if existing.len() != plan.len()
                || existing.iter().zip(plan).any(|(stored, expected)| {
                    stored.part_index != expected.part_index as u64
                        || stored.token_hash != expected.token_hash
                        || stored.token_start != expected.token_start as u64
                        || stored.token_end != expected.token_end as u64
                })
            {
                anyhow::bail!("deterministic token plan changed for operation '{operation_id}'");
            }
            return Ok(());
        }

        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        let rows = plan
            .iter()
            .map(|part| {
                let key = format!("{}-{:08}", record_key(operation_id), part.part_index);
                DbOperationPart {
                    id: Some(RecordId::new("memory_operation_part", key)),
                    operation_id: operation_id.to_owned(),
                    part_index: part.part_index as u64,
                    token_start: part.token_start as u64,
                    token_end: part.token_end as u64,
                    token_count: part.token_count as u64,
                    token_hash: part.token_hash.clone(),
                    content: part.content.clone(),
                    state: "planned".to_owned(),
                    embedding: None,
                    updated_at: Datetime::default(),
                }
            })
            .collect::<Vec<_>>();
        self.await_database(
            "plan persistence",
            connection.generation,
            db.query(
                "BEGIN TRANSACTION;\n\
             INSERT INTO memory_operation_part $parts;\n\
             COMMIT TRANSACTION;",
            )
            .bind(("parts", rows)),
        )
        .await?
        .check()?;
        Ok(())
    }

    async fn mark_part_indexed(
        &self,
        operation_id: &str,
        part_index: u64,
        embedding: Vec<f32>,
    ) -> Result<()> {
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        self.await_database(
            "part persistence",
            connection.generation,
            db.query(
            "UPDATE memory_operation_part SET state = 'indexed', embedding = $embedding, updated_at = $now WHERE operation_id = $id AND part_index = $index",
        )
        .bind(("embedding", embedding))
        .bind(("now", Datetime::default()))
        .bind(("id", operation_id.to_owned()))
        .bind(("index", part_index)),
        )
        .await?
        .check()?;
        Ok(())
    }

    async fn record_processing_error(&self, operation_id: &str, error: &anyhow::Error) {
        let message = bounded_error(&format!("{error:#}"));
        let current = match self.get(operation_id).await {
            Ok(Some(receipt)) if !receipt.state.is_terminal() => receipt,
            _ => return,
        };
        let _ = self
            .transition(
                operation_id,
                current.state,
                current.blocked_by,
                current.result,
                Some(message.clone()),
                Some(json!({"paused":true,"error":message})),
            )
            .await;
    }

    async fn record_executor_events(&self, mut events: broadcast::Receiver<ExecutorEvent>) {
        loop {
            match events.recv().await {
                Ok(event) => {
                    // The child emits `working` every 250 ms so the in-memory
                    // watchdog can distinguish slow inference from a frozen
                    // process. Persisting every heartbeat turns a long model
                    // load into four SurrealDB transactions per second and can
                    // starve the operation reads needed to finish that same
                    // request. Request acceptance, completion, errors, exits,
                    // and watchdog failures remain durable journal records.
                    if event.kind == ExecutorEventKind::Progress
                        && event.message.as_deref() == Some("working")
                    {
                        continue;
                    }
                    if let Err(error) = self.persist_executor_event(event).await {
                        tracing::error!(%error, "executor journal write failed");
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::error!(skipped, "executor journal receiver lagged");
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    async fn persist_executor_event(&self, event: ExecutorEvent) -> Result<()> {
        let Some(operation_id) = event.operation_id else {
            return Ok(());
        };
        if self.get(&operation_id).await?.is_none() {
            return Ok(());
        }
        let kind = serde_json::to_value(&event.kind)?
            .as_str()
            .context("executor event kind must serialize as a string")?
            .to_owned();
        let row = DbExecutorEvent {
            id: None,
            operation_id: operation_id.clone(),
            generation: event.generation,
            progress_seq: event.progress_seq,
            kind,
            message: event.message,
            occurred_at: Datetime::default(),
        };
        let key = format!(
            "{}-{:016}-{:016}",
            record_key(&operation_id),
            event.generation,
            event.progress_seq
        );
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        self.await_database(
            "executor event persistence",
            connection.generation,
            db.query("CREATE type::record('memory_executor_event', $key) CONTENT $event")
                .bind(("key", key))
                .bind(("event", row)),
        )
        .await?
        .check()?;
        Ok(())
    }

    async fn persist_executor_snapshot(&self, operation_id: &str) -> Result<()> {
        let Some(snapshot) = self
            .embedding_service
            .executor_snapshot_for_operation(operation_id)
        else {
            return Ok(());
        };
        let connection = self.ledger_connection().await?;
        let db = connection.db.clone();
        self.await_database(
            "executor snapshot persistence",
            connection.generation,
            db.query(
                "UPDATE memory_operation SET executor_generation = $generation, executor_progress_seq = $progress, executor_exit_count = $exit_count, executor_last_exit = $last_exit, executor_error = $executor_error, updated_at = $now WHERE operation_id = $id",
            )
            .bind(("generation", snapshot.generation))
            .bind(("progress", snapshot.progress_seq))
            .bind(("exit_count", snapshot.exit_count))
            .bind(("last_exit", snapshot.last_exit))
            .bind(("executor_error", snapshot.error))
            .bind(("now", Datetime::default()))
            .bind(("id", operation_id.to_owned())),
        )
            .await?
            .check()?;
        Ok(())
    }

    async fn process_pending(&self, operation_id: &str) -> Result<()> {
        #[cfg(test)]
        if let Some(process) = &self.process_override {
            return process(operation_id.to_owned()).await;
        }
        self.process(operation_id).await
    }

    async fn drain_pending(&self, initial: Vec<String>) {
        let mut pending = VecDeque::from(initial);
        let mut queued = pending.iter().cloned().collect::<HashSet<_>>();
        let mut retried = HashSet::new();
        let mut rescan_after_wave = false;

        loop {
            while let Some(operation_id) = pending.pop_front() {
                queued.remove(&operation_id);
                if let Err(error) = self.process_pending(&operation_id).await {
                    if is_recovered_ledger_deadline(&error)
                        && retried.insert(operation_id.clone())
                        && queued.insert(operation_id.clone())
                    {
                        tracing::warn!(%operation_id, "retrying stale-ledger interruption once");
                        pending.push_back(operation_id);
                        continue;
                    }
                    tracing::error!(%operation_id, %error, "operation processing paused");
                    self.record_processing_error(&operation_id, &error).await;
                    continue;
                }
                let committed_now = match self.get(&operation_id).await {
                    Ok(Some(receipt)) => receipt.state == OperationState::Committed,
                    Ok(None) => false,
                    Err(error) => {
                        tracing::error!(%operation_id, %error, "operation post-process read failed");
                        false
                    }
                };
                rescan_after_wave |= committed_now;
            }

            if !rescan_after_wave {
                break;
            }
            rescan_after_wave = false;
            match self.list_nonterminal_ids().await {
                Ok(ids) => {
                    for id in ids {
                        if queued.insert(id.clone()) {
                            pending.push_back(id);
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "dependent operation reconciliation failed");
                    break;
                }
            }
        }
    }

    async fn reconcile_nonterminal(&self) -> Result<usize> {
        let ids = self.list_nonterminal_ids().await?;
        let count = ids.len();
        self.drain_pending(ids).await;
        Ok(count)
    }

    async fn run(self, mut wake_rx: mpsc::Receiver<String>) {
        let (reconciliation, retried) =
            retry_recovered_ledger_once(|| self.reconcile_nonterminal()).await;
        match reconciliation {
            Ok(_) if retried => {
                tracing::warn!("operation startup reconciliation recovered after one retry")
            }
            Ok(_) => {}
            Err(error) if retried => {
                tracing::error!(%error, "operation startup reconciliation retry failed");
            }
            Err(error) => tracing::error!(%error, "operation startup reconciliation failed"),
        }

        while let Some(operation_id) = wake_rx.recv().await {
            self.drain_pending(vec![operation_id]).await;
        }
    }

    async fn process(&self, operation_id: &str) -> Result<()> {
        let Some(operation) = self.get_db(operation_id).await? else {
            return Ok(());
        };
        let mut state = parse_state(&operation.state)?;
        if state.is_terminal() {
            return Ok(());
        }

        let blockers = self.dependency_blockers(&operation).await?;
        if !blockers.is_empty() {
            if state != OperationState::Blocked || operation.blocked_by != blockers {
                self.transition(
                    operation_id,
                    OperationState::Blocked,
                    blockers,
                    None,
                    None,
                    Some(json!({"reason":"dependencies_not_committed"})),
                )
                .await?;
            }
            return Ok(());
        }

        if matches!(state, OperationState::Accepted | OperationState::Blocked) {
            if let Err(error) = validate_payload(&operation.kind, &operation.payload) {
                self.transition(
                    operation_id,
                    OperationState::Rejected,
                    Vec::new(),
                    None,
                    Some(error.to_string()),
                    Some(json!({"contract":"rejected"})),
                )
                .await?;
                return Ok(());
            }
            self.transition(
                operation_id,
                OperationState::Validated,
                Vec::new(),
                None,
                None,
                Some(json!({"contract":"validated"})),
            )
            .await?;
            state = OperationState::Validated;
        }

        if operation.kind == "add_memory" {
            self.process_add_memory(&operation, state).await?;
        } else {
            let result = self.process_task_operation(&operation, state).await?;
            self.finish_operation(operation_id, result).await?;
        }

        Ok(())
    }

    pub async fn event_stream(
        &self,
        operation_id: &str,
        after: u64,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<OperationEvent>> + Send>>> {
        // Subscribe before reading the durable history. A racing transition is
        // present in either the snapshot or the receiver, and sequence checks
        // remove the harmless overlap.
        let mut live = self.events_tx.subscribe();
        let history = self.events_after(operation_id, after).await?;
        let service = self.clone();
        let operation_id = operation_id.to_owned();
        let stream = async_stream::try_stream! {
            let mut last_sequence = after;
            for event in history {
                if event.sequence > last_sequence {
                    last_sequence = event.sequence;
                    yield event;
                }
            }

            loop {
                match live.recv().await {
                    Ok(event)
                        if event.operation_id == operation_id
                            && event.sequence > last_sequence =>
                    {
                        last_sequence = event.sequence;
                        yield event;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for event in service.events_after(&operation_id, last_sequence).await? {
                            if event.sequence > last_sequence {
                                last_sequence = event.sequence;
                                yield event;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };
        Ok(Box::pin(stream))
    }

    async fn process_add_memory(
        &self,
        operation: &DbOperation,
        mut state: OperationState,
    ) -> Result<()> {
        let request: AddMemoryRequest = serde_json::from_value(operation.payload.clone())
            .context("invalid add_memory payload")?;
        self.embedding_service
            .prepare_operation(
                &operation.operation_id,
                &ExecutorSnapshot {
                    generation: operation.executor_generation,
                    progress_seq: operation.executor_progress_seq,
                    exit_count: operation.executor_exit_count,
                    last_exit: operation.executor_last_exit.clone(),
                    error: operation.executor_error.clone(),
                },
            )
            .await?;
        let mut stored_parts = self.operation_parts(&operation.operation_id).await?;
        if stored_parts.is_empty() {
            let planned = self
                .embedding_service
                .plan_for_operation(&operation.operation_id, &request.content)
                .await;
            self.persist_executor_snapshot(&operation.operation_id)
                .await?;
            let plan = planned?;
            if plan.is_empty() {
                anyhow::bail!("embedding planner returned no parts");
            }
            self.persist_plan(&operation.operation_id, &plan).await?;
            stored_parts = self.operation_parts(&operation.operation_id).await?;
        }
        validate_stored_plan(&stored_parts)?;

        if state == OperationState::Validated {
            self.transition(
                &operation.operation_id,
                OperationState::Planned,
                Vec::new(),
                None,
                None,
                Some(json!({
                    "parts": stored_parts.len(),
                    "token_count": stored_parts.iter().map(|part| part.token_end).max().unwrap_or(0),
                    "planner": "active_model_tokenizer"
                })),
            )
            .await?;
            state = OperationState::Planned;
        }
        if state == OperationState::Planned {
            self.transition(
                &operation.operation_id,
                OperationState::Processing,
                Vec::new(),
                None,
                None,
                Some(json!({"executor_generation":operation.executor_generation})),
            )
            .await?;
            state = OperationState::Processing;
        }

        if state == OperationState::Processing {
            for part in &mut stored_parts {
                if part.state == "indexed" && part.embedding.is_some() {
                    continue;
                }
                let embedded = self
                    .embedding_service
                    .embed_for_operation(
                        &operation.operation_id,
                        part.part_index as usize,
                        &part.content,
                    )
                    .await;
                self.persist_executor_snapshot(&operation.operation_id)
                    .await?;
                let embedding = embedded?;
                self.mark_part_indexed(&operation.operation_id, part.part_index, embedding.clone())
                    .await?;
                part.embedding = Some(embedding);
                part.state = "indexed".to_owned();
            }
        }

        // Re-read the authoritative part rows after each restart. Every part
        // must be explicitly indexed before the logical memory can commit.
        stored_parts = self.operation_parts(&operation.operation_id).await?;
        if stored_parts
            .iter()
            .any(|part| part.state != "indexed" || part.embedding.is_none())
        {
            anyhow::bail!("operation parts are not fully indexed");
        }
        let aggregate = aggregate_embeddings(&stored_parts)?;
        let exact_token_count = stored_parts
            .iter()
            .map(|part| part.token_end)
            .max()
            .unwrap_or(0);
        let token_count = if exact_token_count == 0 {
            None
        } else {
            Some(
                exact_token_count
                    .try_into()
                    .context("logical memory token count exceeds u32")?,
            )
        };
        let memory = request.into_memory();
        let stored = self
            .surreal()?
            .store_indexed_memory(
                &record_key(&operation.operation_id),
                memory,
                aggregate,
                token_count,
            )
            .await?;
        self.finish_operation(&operation.operation_id, serde_json::to_value(stored)?)
            .await
    }

    async fn process_task_operation(
        &self,
        operation: &DbOperation,
        mut state: OperationState,
    ) -> Result<Value> {
        if state == OperationState::Validated {
            self.transition(
                &operation.operation_id,
                OperationState::Planned,
                Vec::new(),
                None,
                None,
                Some(json!({"scheduler":"dependency_ordered"})),
            )
            .await?;
            state = OperationState::Planned;
        }
        if state == OperationState::Planned {
            self.transition(
                &operation.operation_id,
                OperationState::Processing,
                Vec::new(),
                None,
                None,
                None,
            )
            .await?;
        }

        match operation.kind.as_str() {
            "create_task_stream" => {
                let payload: CreateTaskStreamPayload =
                    serde_json::from_value(operation.payload.clone())
                        .context("invalid create_task_stream payload")?;
                if let Some(existing) = self
                    .storage
                    .get_task_stream(
                        &payload.name,
                        payload.user_id.as_deref(),
                        payload.agent_id.as_deref(),
                    )
                    .await?
                {
                    return Ok(serde_json::to_value(existing)?);
                }
                Ok(serde_json::to_value(
                    self.storage
                        .create_task_stream(TaskStream::new(
                            payload.name,
                            payload.description,
                            payload.agent_id,
                            payload.user_id,
                        ))
                        .await?,
                )?)
            }
            "add_task_step" => {
                let payload: AddTaskStepPayload = serde_json::from_value(operation.payload.clone())
                    .context("invalid add_task_step payload")?;
                Ok(serde_json::to_value(
                    self.storage
                        .add_task_step(
                            &payload.stream_name,
                            payload.user_id.as_deref(),
                            payload.agent_id.as_deref(),
                            TaskStep::new(
                                payload.ordinal,
                                payload.name,
                                payload.description,
                                payload.idempotency_key,
                            ),
                        )
                        .await?,
                )?)
            }
            "complete_step" => {
                let payload: CompleteStepPayload =
                    serde_json::from_value(operation.payload.clone())
                        .context("invalid complete_step payload")?;
                Ok(serde_json::to_value(
                    self.storage
                        .complete_step(&payload.idempotency_key, payload.result)
                        .await?,
                )?)
            }
            _ => unreachable!("validated operation kind"),
        }
    }

    async fn finish_operation(&self, operation_id: &str, result: Value) -> Result<()> {
        let current = self
            .get(operation_id)
            .await?
            .with_context(|| format!("operation '{operation_id}' disappeared"))?;
        if current.state == OperationState::Committed {
            return Ok(());
        }
        if current.state != OperationState::Indexed {
            self.transition(
                operation_id,
                OperationState::Indexed,
                Vec::new(),
                Some(result.clone()),
                None,
                None,
            )
            .await?;
        }
        self.transition(
            operation_id,
            OperationState::Committed,
            Vec::new(),
            Some(result),
            None,
            Some(json!({"outcome":"committed"})),
        )
        .await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct CreateTaskStreamPayload {
    name: String,
    description: Option<String>,
    agent_id: Option<String>,
    user_id: Option<String>,
}

impl CreateTaskStreamPayload {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("task stream name cannot be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct AddTaskStepPayload {
    stream_name: String,
    #[serde(default = "default_ordinal")]
    ordinal: u32,
    name: String,
    description: Option<String>,
    idempotency_key: String,
    agent_id: Option<String>,
    user_id: Option<String>,
}

fn default_ordinal() -> u32 {
    1
}

impl AddTaskStepPayload {
    fn validate(&self) -> Result<()> {
        if self.stream_name.trim().is_empty()
            || self.name.trim().is_empty()
            || self.idempotency_key.trim().is_empty()
        {
            anyhow::bail!("stream_name, name, and idempotency_key cannot be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct CompleteStepPayload {
    idempotency_key: String,
    result: Option<String>,
}

fn validate_payload(kind: &str, payload: &Value) -> Result<()> {
    match kind {
        "add_memory" => {
            let request: AddMemoryRequest =
                serde_json::from_value(payload.clone()).context("invalid add_memory payload")?;
            request.validate().map_err(anyhow::Error::from)
        }
        "create_task_stream" => {
            let request: CreateTaskStreamPayload = serde_json::from_value(payload.clone())
                .context("invalid create_task_stream payload")?;
            request.validate()
        }
        "add_task_step" => {
            let request: AddTaskStepPayload =
                serde_json::from_value(payload.clone()).context("invalid add_task_step payload")?;
            request.validate()
        }
        "complete_step" => {
            let request: CompleteStepPayload =
                serde_json::from_value(payload.clone()).context("invalid complete_step payload")?;
            if request.idempotency_key.trim().is_empty() {
                anyhow::bail!("idempotency_key cannot be empty");
            }
            Ok(())
        }
        other => anyhow::bail!("unknown operation kind '{other}'"),
    }
}

fn validate_stored_plan(parts: &[DbOperationPart]) -> Result<()> {
    if parts.is_empty() {
        anyhow::bail!("embedding plan is empty");
    }
    for (expected, part) in parts.iter().enumerate() {
        if part.part_index != expected as u64
            || part.token_end < part.token_start
            || part.token_count != part.token_end - part.token_start
            || part.token_hash.trim().is_empty()
        {
            anyhow::bail!("persisted embedding plan is not contiguous and self-consistent");
        }
    }
    Ok(())
}

fn aggregate_embeddings(parts: &[DbOperationPart]) -> Result<Vec<f32>> {
    let dimensions = parts
        .first()
        .and_then(|part| part.embedding.as_ref())
        .map(Vec::len)
        .context("cannot aggregate an empty embedding plan")?;
    let mut aggregate = vec![0.0f32; dimensions];
    for part in parts {
        let embedding = part
            .embedding
            .as_ref()
            .context("cannot aggregate an unindexed part")?;
        if embedding.len() != dimensions {
            anyhow::bail!("embedding part dimensions do not match");
        }
        for (target, value) in aggregate.iter_mut().zip(embedding) {
            *target += value;
        }
    }
    let divisor = parts.len() as f32;
    for value in &mut aggregate {
        *value /= divisor;
    }
    let norm = aggregate
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if norm > 0.0 {
        for value in &mut aggregate {
            *value /= norm;
        }
    }
    Ok(aggregate)
}

fn bounded_error(error: &str) -> String {
    error.chars().take(2_000).collect()
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after: u64,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(submit_operation))
        .route("/{id}", get(get_operation))
        .route("/{id}/events", get(operation_events))
}

async fn submit_operation(
    State(state): State<AppState>,
    Json(request): Json<OperationRequest>,
) -> Result<(StatusCode, Json<OperationReceipt>), ApiFailure> {
    match state.operations.submit(request).await {
        Ok((receipt, created)) => Ok((
            if created {
                StatusCode::ACCEPTED
            } else {
                StatusCode::OK
            },
            Json(receipt),
        )),
        Err(SubmitError::Invalid(error)) => Err(bad_request(error.to_string())),
        Err(SubmitError::Conflict(existing)) => Err((
            StatusCode::CONFLICT,
            Json(ApiError {
                error: format!(
                    "operation_id '{}' already exists with payload_hash {}",
                    existing.operation_id, existing.payload_hash
                ),
            }),
        )),
        Err(SubmitError::Storage(error)) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: error.to_string(),
            }),
        )),
    }
}

async fn get_operation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<OperationReceipt>, ApiFailure> {
    let receipt = state
        .operations
        .get(&id)
        .await
        .map_err(crate::api::api_error)?
        .ok_or_else(|| not_found(format!("operation '{id}' not found")))?;
    Ok(Json(receipt))
}

async fn operation_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiFailure> {
    if state
        .operations
        .get(&id)
        .await
        .map_err(crate::api::api_error)?
        .is_none()
    {
        return Err(not_found(format!("operation '{id}' not found")));
    }

    let events = state
        .operations
        .event_stream(&id, query.after)
        .await
        .map_err(crate::api::api_error)?;
    let stream = events.map(|item| {
        let event = match item {
            Ok(event) => event,
            Err(error) => {
                return Ok(Event::default()
                    .event("ledger_error")
                    .data(bounded_error(&format!("{error:#}"))));
            }
        };
        let sequence = event.sequence.to_string();
        let data = serde_json::to_string(&event).expect("operation event serializes");
        Ok(Event::default()
            .event("operation_state")
            .id(sequence)
            .data(data))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn payload_hash(payload: &Value) -> Result<String> {
    let canonical = serde_json::to_vec(payload).context("payload serialization failed")?;
    Ok(digest_hex(&canonical))
}

fn record_key(operation_id: &str) -> String {
    digest_hex(operation_id.as_bytes())
}

fn digest_hex(input: &[u8]) -> String {
    Sha256::digest(input)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_state(state: &str) -> Result<OperationState> {
    match state {
        "accepted" => Ok(OperationState::Accepted),
        "validated" => Ok(OperationState::Validated),
        "blocked" => Ok(OperationState::Blocked),
        "planned" => Ok(OperationState::Planned),
        "processing" => Ok(OperationState::Processing),
        "indexed" => Ok(OperationState::Indexed),
        "committed" => Ok(OperationState::Committed),
        "rejected" => Ok(OperationState::Rejected),
        other => anyhow::bail!("unknown operation state '{other}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    struct NoOpEmbedder;

    #[async_trait::async_trait]
    impl EmbeddingService for NoOpEmbedder {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.5; 8])
        }

        async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(texts.into_iter().map(|_| vec![0.5; 8]).collect())
        }

        fn dimensions(&self) -> usize {
            8
        }
    }

    struct PlannedEmbedder {
        fail_at_call: Option<usize>,
        calls: AtomicUsize,
    }

    impl PlannedEmbedder {
        fn completing() -> Self {
            Self {
                fail_at_call: None,
                calls: AtomicUsize::new(0),
            }
        }

        fn failing_at(call: usize) -> Self {
            Self {
                fail_at_call: Some(call),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl EmbeddingService for PlannedEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_at_call == Some(call) {
                anyhow::bail!("fixture executor exit at call {call}");
            }
            Ok(vec![text.len() as f32 + 1.0, call as f32 + 1.0])
        }

        async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            let mut result = Vec::with_capacity(texts.len());
            for text in texts {
                result.push(self.embed(&text).await?);
            }
            Ok(result)
        }

        async fn plan(&self, _text: &str) -> Result<Vec<EmbeddingPlanPart>> {
            Ok(["alpha", "beta", "gamma"]
                .into_iter()
                .enumerate()
                .map(|(part_index, content)| EmbeddingPlanPart {
                    part_index,
                    token_start: part_index * 3,
                    token_end: (part_index + 1) * 3,
                    token_count: 3,
                    token_hash: digest_hex(content.as_bytes()),
                    content: content.to_owned(),
                })
                .collect())
        }

        fn dimensions(&self) -> usize {
            2
        }
    }

    fn service_without_coordinator(
        storage: Arc<SurrealStorage>,
        embedder: Arc<dyn EmbeddingService>,
        ledger_connector: LedgerConnector,
        initial_connection: Option<LedgerConnection>,
        query_timeout: Duration,
    ) -> OperationService {
        let (wake_tx, _) = mpsc::channel(4);
        let (events_tx, _) = broadcast::channel(4);
        OperationService {
            storage: storage as Arc<dyn MemoryStorage>,
            embedding_service: embedder,
            query_timeout,
            ledger_connector,
            ledger_connection: Arc::new(ArcSwapOption::new(initial_connection.map(Arc::new))),
            connection_replacement: Arc::new(tokio::sync::Mutex::new(())),
            wake_tx,
            events_tx,
            process_override: None,
        }
    }

    fn deadline_error(recovered: bool) -> anyhow::Error {
        LedgerDeadlineExceeded {
            stage: "fixture",
            timeout_ms: 1,
            recovered,
        }
        .into()
    }

    #[tokio::test]
    async fn concurrent_first_use_publishes_one_ledger_generation() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let database = storage.db().unwrap();
        let connect_calls = Arc::new(AtomicUsize::new(0));
        let ledger_connector: LedgerConnector = {
            let connect_calls = Arc::clone(&connect_calls);
            Arc::new(move || {
                let database = database.clone();
                let connect_calls = Arc::clone(&connect_calls);
                Box::pin(async move {
                    connect_calls.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    Ok(database)
                })
            })
        };
        let service = service_without_coordinator(
            Arc::clone(&storage),
            embedder,
            ledger_connector,
            None,
            Duration::from_secs(1),
        );

        let connections =
            futures_util::future::join_all((0..4).map(|_| service.ledger_connection()))
                .await
                .into_iter()
                .collect::<Result<Vec<_>>>()
                .unwrap();

        assert_eq!(connect_calls.load(Ordering::SeqCst), 1);
        assert!(
            connections
                .iter()
                .all(|connection| connection.generation == 0)
        );
        assert!(
            connections
                .iter()
                .all(|connection| Arc::ptr_eq(connection, &connections[0]))
        );
    }

    #[tokio::test]
    async fn overlapping_replacements_publish_only_the_next_generation() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let database = storage.db().unwrap();
        let connect_calls = Arc::new(AtomicUsize::new(0));
        let ledger_connector: LedgerConnector = {
            let connect_calls = Arc::clone(&connect_calls);
            let replacement = database.clone();
            Arc::new(move || {
                let replacement = replacement.clone();
                let connect_calls = Arc::clone(&connect_calls);
                Box::pin(async move {
                    connect_calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Ok(replacement)
                })
            })
        };
        let service = service_without_coordinator(
            Arc::clone(&storage),
            embedder,
            ledger_connector,
            Some(LedgerConnection {
                generation: 0,
                db: database,
            }),
            Duration::from_secs(1),
        );

        let replacements =
            futures_util::future::join_all((0..4).map(|_| service.replace_ledger_connection(0)))
                .await;

        assert!(replacements.into_iter().all(|result| result.is_ok()));
        assert_eq!(connect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(service.ledger_connection().await.unwrap().generation, 1);
    }

    #[tokio::test]
    async fn replacement_failure_is_not_a_recovered_ledger_deadline() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let database = storage.db().unwrap();
        let connect_calls = Arc::new(AtomicUsize::new(0));
        let ledger_connector: LedgerConnector = {
            let connect_calls = Arc::clone(&connect_calls);
            Arc::new(move || {
                let connect_calls = Arc::clone(&connect_calls);
                Box::pin(async move {
                    connect_calls.fetch_add(1, Ordering::SeqCst);
                    anyhow::bail!("fixture replacement failed")
                })
            })
        };
        let service = service_without_coordinator(
            Arc::clone(&storage),
            embedder,
            ledger_connector,
            Some(LedgerConnection {
                generation: 0,
                db: database,
            }),
            Duration::from_millis(1),
        );

        let error = service
            .await_database(
                "fixture",
                0,
                std::future::pending::<std::result::Result<(), anyhow::Error>>(),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "operation database fixture timed out after 1ms"
        );
        assert!(!is_recovered_ledger_deadline(&error));
        assert_eq!(connect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(service.ledger_connection().await.unwrap().generation, 0);
    }

    #[tokio::test]
    async fn startup_retry_runs_once_only_for_a_recovered_ledger_deadline() {
        let recovered_calls = Arc::new(AtomicUsize::new(0));
        let recovered_counter = Arc::clone(&recovered_calls);
        let (recovered_result, retried) = retry_recovered_ledger_once(move || {
            let attempt = recovered_counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    Err(deadline_error(true))
                } else {
                    Ok("recovered")
                }
            }
        })
        .await;
        assert_eq!(recovered_result.unwrap(), "recovered");
        assert!(retried);
        assert_eq!(recovered_calls.load(Ordering::SeqCst), 2);

        for first_error in [deadline_error(false), anyhow::anyhow!("executor failed")] {
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&calls);
            let mut first_error = Some(first_error);
            let (result, retried) = retry_recovered_ledger_once(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                let error = first_error.take().expect("only one attempt");
                async move { Err::<(), _>(error) }
            })
            .await;
            assert!(result.is_err());
            assert!(!retried);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }

        let repeated_calls = Arc::new(AtomicUsize::new(0));
        let repeated_counter = Arc::clone(&repeated_calls);
        let (result, retried) = retry_recovered_ledger_once(move || {
            repeated_counter.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(deadline_error(true)) }
        })
        .await;
        assert!(result.is_err());
        assert!(retried);
        assert_eq!(repeated_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn coordinator_drain_retries_only_a_recovered_ledger_deadline_once() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let database = storage.db().unwrap();

        let make_service = |process_override: ProcessOverride| {
            let replacement = database.clone();
            let ledger_connector: LedgerConnector = Arc::new(move || {
                let replacement = replacement.clone();
                Box::pin(async move { Ok(replacement) })
            });
            let mut service = service_without_coordinator(
                Arc::clone(&storage),
                Arc::clone(&embedder),
                ledger_connector,
                Some(LedgerConnection {
                    generation: 0,
                    db: database.clone(),
                }),
                Duration::from_secs(1),
            );
            service.process_override = Some(process_override);
            service
        };

        let recovered_then_executor_calls = Arc::new(AtomicUsize::new(0));
        let recovered_counter = Arc::clone(&recovered_then_executor_calls);
        let recovered_then_executor: ProcessOverride = Arc::new(move |_| {
            let attempt = recovered_counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if attempt == 0 {
                    Err(deadline_error(true))
                } else {
                    anyhow::bail!("fixture executor failure")
                }
            })
        });
        make_service(recovered_then_executor)
            .drain_pending(vec!["recovered-then-executor".to_owned()])
            .await;
        assert_eq!(recovered_then_executor_calls.load(Ordering::SeqCst), 2);

        let repeated_deadline_calls = Arc::new(AtomicUsize::new(0));
        let repeated_counter = Arc::clone(&repeated_deadline_calls);
        let repeated_deadline: ProcessOverride = Arc::new(move |_| {
            repeated_counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(deadline_error(true)) })
        });
        make_service(repeated_deadline)
            .drain_pending(vec!["repeated-deadline".to_owned()])
            .await;
        assert_eq!(repeated_deadline_calls.load(Ordering::SeqCst), 2);

        let executor_calls = Arc::new(AtomicUsize::new(0));
        let executor_counter = Arc::clone(&executor_calls);
        let executor_failure: ProcessOverride = Arc::new(move |_| {
            executor_counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { anyhow::bail!("fixture executor failure") })
        });
        make_service(executor_failure)
            .drain_pending(vec!["executor-only".to_owned()])
            .await;
        assert_eq!(executor_calls.load(Ordering::SeqCst), 1);
    }

    async fn test_router() -> (Router, OperationService) {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage: Arc<dyn MemoryStorage> = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let operations = OperationService::start(Arc::clone(&storage), Arc::clone(&embedder));
        let state = AppState {
            storage,
            embedding_service: embedder,
            operations: operations.clone(),
        };
        (
            Router::new()
                .nest("/api/v2/operations", router())
                .with_state(state),
            operations,
        )
    }

    fn operation_request(id: &str, kind: &str, dependencies: Vec<&str>, payload: Value) -> Value {
        json!({
            "operation_id": id,
            "schema_version": 2,
            "kind": kind,
            "dependencies": dependencies,
            "payload_hash": payload_hash(&payload).unwrap(),
            "payload": payload
        })
    }

    fn openapi_spec() -> Value {
        serde_json::from_str(include_str!("../openapi/surreal-memory-v2.openapi.json"))
            .expect("OpenAPI document must be valid JSON")
    }

    async fn post(router: Router, body: Value) -> axum::response::Response {
        router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v2/operations")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn get_receipt(router: Router, id: &str) -> OperationReceipt {
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v2/operations/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    async fn wait_for_state(
        router: Router,
        service: &OperationService,
        id: &str,
        expected: OperationState,
    ) {
        let mut events = service.events_tx.subscribe();
        loop {
            let receipt = get_receipt(router.clone(), id).await;
            if receipt.state == expected {
                return;
            }
            match events.recv().await {
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("operation event stream closed before {id} reached {expected:?}")
                }
            }
        }
    }

    async fn wait_for_service_state(
        service: &OperationService,
        id: &str,
        expected: OperationState,
        require_error: bool,
    ) -> OperationReceipt {
        let mut events = service.events_tx.subscribe();
        loop {
            let receipt = service.get(id).await.unwrap().unwrap();
            if receipt.state == expected && (!require_error || receipt.error.is_some()) {
                return receipt;
            }
            match events.recv().await {
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("operation event stream closed before {id} reached {expected:?}")
                }
            }
        }
    }

    #[test]
    fn canonical_payload_hash_is_stable() {
        let payload = json!({"content":"delta","user_id":"project"});
        assert_eq!(
            payload_hash(&payload).unwrap(),
            payload_hash(&payload).unwrap()
        );
    }

    #[test]
    fn openapi_examples_match_serialized_request_and_receipt_contracts() {
        let spec = openapi_spec();
        assert_eq!(spec["openapi"], "3.1.0");
        assert_eq!(spec["info"]["version"], "1.8.0");

        let request_value = spec["components"]["examples"]["AddMemoryRequest"]["value"].clone();
        let request: OperationRequest =
            serde_json::from_value(request_value.clone()).expect("request example");
        request.validate().expect("request example must validate");
        assert_eq!(
            payload_hash(&request.payload).expect("canonical payload hash"),
            request.payload_hash
        );
        assert_eq!(
            serde_json::to_value(request).expect("serialize request"),
            request_value
        );

        let receipt_value = spec["components"]["examples"]["CommittedReceipt"]["value"].clone();
        let receipt: OperationReceipt =
            serde_json::from_value(receipt_value.clone()).expect("receipt example");
        assert_eq!(
            serde_json::to_value(receipt).expect("serialize receipt"),
            receipt_value
        );

        let long_value = spec["paths"]["/api/v2/operations"]["post"]["requestBody"]
            ["content"]["application/json"]["examples"]["longLogicalMemory"]["value"]
            .clone();
        let long_request: OperationRequest =
            serde_json::from_value(long_value).expect("long-memory example");
        long_request
            .validate()
            .expect("long-memory example must use its canonical hash");

        for status in ["200", "202", "400", "409", "503"] {
            assert!(
                spec["paths"]["/api/v2/operations"]["post"]["responses"]
                    .get(status)
                    .is_some(),
                "OpenAPI POST response {status} is missing"
            );
        }
        for status in ["200", "404", "503"] {
            assert!(
                spec["paths"]["/api/v2/operations/{operation_id}"]["get"]["responses"]
                    .get(status)
                    .is_some(),
                "OpenAPI GET response {status} is missing"
            );
        }
    }

    #[tokio::test]
    async fn openapi_replay_and_conflict_examples_match_http_statuses() {
        let spec = openapi_spec();
        let request = spec["components"]["examples"]["AddMemoryRequest"]["value"].clone();
        let id = request["operation_id"].as_str().expect("operation id");
        let (router, service) = test_router().await;

        let created = post(router.clone(), request.clone()).await;
        assert_eq!(created.status(), StatusCode::ACCEPTED);
        assert!(spec["paths"]["/api/v2/operations"]["post"]["responses"]["202"].is_object());
        wait_for_state(router.clone(), &service, id, OperationState::Committed).await;

        let replay = post(router.clone(), request.clone()).await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replay_receipt: OperationReceipt =
            serde_json::from_slice(&to_bytes(replay.into_body(), usize::MAX).await.unwrap())
                .expect("replay receipt");
        assert_eq!(replay_receipt.state, OperationState::Committed);

        let mut conflict = request;
        conflict["payload"] =
            json!({"content":"Different payload","user_id":"prometheus-skill-pack"});
        conflict["payload_hash"] = Value::String(payload_hash(&conflict["payload"]).unwrap());
        let conflict_response = post(router, conflict).await;
        assert_eq!(conflict_response.status(), StatusCode::CONFLICT);
        assert!(spec["paths"]["/api/v2/operations"]["post"]["responses"]["409"].is_object());
    }

    #[test]
    fn request_rejects_hash_mismatch_and_unknown_kind() {
        let mut request = OperationRequest {
            operation_id: "op-1".into(),
            schema_version: 2,
            kind: "add_memory".into(),
            dependencies: Vec::new(),
            payload_hash: "wrong".into(),
            payload: json!({"content":"delta"}),
        };
        assert!(
            request
                .validate()
                .unwrap_err()
                .to_string()
                .contains("payload_hash mismatch")
        );
        request.payload_hash = payload_hash(&request.payload).unwrap();
        request.kind = "unknown".into();
        assert!(
            request
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unknown operation kind")
        );
    }

    #[tokio::test]
    async fn concurrent_duplicate_submission_returns_one_receipt() {
        let (router, service) = test_router().await;
        let body = operation_request(
            "same-id",
            "add_memory",
            vec![],
            json!({"content":"deterministic delta","user_id":"test"}),
        );
        let (left, right) = tokio::join!(
            post(router.clone(), body.clone()),
            post(router.clone(), body)
        );
        assert!(matches!(
            left.status(),
            StatusCode::ACCEPTED | StatusCode::OK
        ));
        assert!(matches!(
            right.status(),
            StatusCode::ACCEPTED | StatusCode::OK
        ));
        wait_for_state(
            router.clone(),
            &service,
            "same-id",
            OperationState::Committed,
        )
        .await;

        let receipt = get_receipt(router, "same-id").await;
        assert_eq!(receipt.operation_id, "same-id");
        assert_eq!(receipt.progress_seq, 6);
    }

    #[tokio::test]
    async fn reusing_operation_id_with_another_hash_is_conflict() {
        let (router, _) = test_router().await;
        let first = operation_request(
            "conflict-id",
            "add_memory",
            vec![],
            json!({"content":"first","user_id":"test"}),
        );
        assert_eq!(
            post(router.clone(), first).await.status(),
            StatusCode::ACCEPTED
        );
        let second = operation_request(
            "conflict-id",
            "add_memory",
            vec![],
            json!({"content":"second","user_id":"test"}),
        );
        assert_eq!(post(router, second).await.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn provider_without_exact_tokenizer_leaves_token_count_unknown() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let service =
            OperationService::start(Arc::clone(&storage) as Arc<dyn MemoryStorage>, embedder);
        let payload = json!({"content":"memory 📚","user_id":"test"});
        service
            .submit(OperationRequest {
                operation_id: "unknown-token-count".to_owned(),
                schema_version: OPERATION_SCHEMA_VERSION,
                kind: "add_memory".to_owned(),
                dependencies: Vec::new(),
                payload_hash: payload_hash(&payload).unwrap(),
                payload,
            })
            .await
            .unwrap();
        wait_for_service_state(
            &service,
            "unknown-token-count",
            OperationState::Committed,
            false,
        )
        .await;

        let stored = storage
            .get_memory(&record_key("unknown-token-count"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.token_count, None);
    }

    #[tokio::test]
    async fn explicit_dependencies_unblock_in_topological_order() {
        let (router, service) = test_router().await;
        let complete = operation_request(
            "op-complete",
            "complete_step",
            vec!["op-step"],
            json!({"idempotency_key":"step-1","result":"done"}),
        );
        let step = operation_request(
            "op-step",
            "add_task_step",
            vec!["op-stream"],
            json!({
                "stream_name":"test-stream",
                "ordinal":1,
                "name":"step 1",
                "description":"work",
                "idempotency_key":"step-1",
                "agent_id":null,
                "user_id":"test"
            }),
        );
        let stream = operation_request(
            "op-stream",
            "create_task_stream",
            vec![],
            json!({
                "name":"test-stream",
                "description":"dependency test",
                "agent_id":null,
                "user_id":"test"
            }),
        );

        assert_eq!(
            post(router.clone(), complete).await.status(),
            StatusCode::ACCEPTED
        );
        assert_eq!(
            post(router.clone(), step).await.status(),
            StatusCode::ACCEPTED
        );
        assert_eq!(
            post(router.clone(), stream).await.status(),
            StatusCode::ACCEPTED
        );
        wait_for_state(
            router.clone(),
            &service,
            "op-stream",
            OperationState::Committed,
        )
        .await;
        wait_for_state(
            router.clone(),
            &service,
            "op-step",
            OperationState::Committed,
        )
        .await;
        wait_for_state(router, &service, "op-complete", OperationState::Committed).await;
    }

    #[tokio::test]
    async fn restart_replays_only_unfinished_parts_and_keeps_one_logical_memory() {
        let first_embedder = Arc::new(PlannedEmbedder::failing_at(1));
        let first_embedding_service: Arc<dyn EmbeddingService> = first_embedder.clone();
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&first_embedding_service))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let first = OperationService::start(
            Arc::clone(&storage) as Arc<dyn MemoryStorage>,
            first_embedding_service,
        );
        let payload = json!({"content":"one logical memory","user_id":"test"});
        first
            .submit(OperationRequest {
                operation_id: "restart-parts".to_owned(),
                schema_version: OPERATION_SCHEMA_VERSION,
                kind: "add_memory".to_owned(),
                dependencies: Vec::new(),
                payload_hash: payload_hash(&payload).unwrap(),
                payload,
            })
            .await
            .unwrap();
        wait_for_service_state(&first, "restart-parts", OperationState::Processing, true).await;
        let interrupted = first.operation_parts("restart-parts").await.unwrap();
        assert_eq!(interrupted.len(), 3);
        assert_eq!(interrupted[0].state, "indexed");
        assert_eq!(interrupted[1].state, "planned");
        assert_eq!(interrupted[2].state, "planned");

        let second_embedder = Arc::new(PlannedEmbedder::completing());
        let second_embedding_service: Arc<dyn EmbeddingService> = second_embedder.clone();
        let second = OperationService::start(
            Arc::clone(&storage) as Arc<dyn MemoryStorage>,
            second_embedding_service,
        );
        wait_for_service_state(&second, "restart-parts", OperationState::Committed, false).await;

        let completed = second.operation_parts("restart-parts").await.unwrap();
        assert!(completed.iter().all(|part| part.state == "indexed"));
        assert!(
            storage
                .get_memory(&record_key("restart-parts"))
                .await
                .unwrap()
                .is_some(),
            "the operation must materialize exactly one stable logical memory key"
        );
        let db = storage.db().unwrap();
        let memory_rows: Vec<Value> = db
            .query("SELECT id FROM memory WHERE id = type::record('memory', $key)")
            .bind(("key", record_key("restart-parts")))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        let history_rows: Vec<Value> = db
            .query("SELECT id FROM memory_history WHERE memory_id = type::record('memory', $key)")
            .bind(("key", record_key("restart-parts")))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(memory_rows.len(), 1, "one logical memory row is committed");
        assert_eq!(
            history_rows.len(),
            1,
            "one logical creation event is committed"
        );
        assert_eq!(
            first_embedder.calls.load(Ordering::SeqCst),
            2,
            "first executor stopped on its second part"
        );
        assert_eq!(
            second_embedder.calls.load(Ordering::SeqCst),
            2,
            "restart embedded only two unfinished parts"
        );
    }

    #[tokio::test]
    async fn startup_reconciliation_exceeds_the_wake_channel_capacity() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let service = OperationService::start_with_capacities(
            Arc::clone(&storage) as Arc<dyn MemoryStorage>,
            embedder,
            Duration::from_secs(10),
            4,
            16,
        );

        for index in 0..12 {
            let payload = json!({
                "name": format!("blocked-{index}"),
                "description": "startup reconciliation fixture",
                "agent_id": null,
                "user_id": "test"
            });
            service
                .submit(OperationRequest {
                    operation_id: format!("startup-{index:02}"),
                    schema_version: OPERATION_SCHEMA_VERSION,
                    kind: "create_task_stream".to_owned(),
                    dependencies: vec!["missing-prerequisite".to_owned()],
                    payload_hash: payload_hash(&payload).unwrap(),
                    payload,
                })
                .await
                .unwrap();
        }

        assert_eq!(service.reconcile_nonterminal().await.unwrap(), 12);
    }

    #[tokio::test]
    async fn startup_reconciliation_query_projects_only_operation_identity() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let service = OperationService::start_with_capacities(
            Arc::clone(&storage) as Arc<dyn MemoryStorage>,
            embedder,
            Duration::from_secs(10),
            4,
            16,
        );
        let sentinel = "payload-must-not-enter-reconciliation-list";
        let payload = json!({
            "name": "projected-reconciliation",
            "description": sentinel.repeat(16_384),
            "agent_id": null,
            "user_id": "test"
        });
        service
            .submit(OperationRequest {
                operation_id: "projected-reconciliation".to_owned(),
                schema_version: OPERATION_SCHEMA_VERSION,
                kind: "create_task_stream".to_owned(),
                dependencies: vec!["missing-prerequisite".to_owned()],
                payload_hash: payload_hash(&payload).unwrap(),
                payload,
            })
            .await
            .unwrap();

        let rows: Vec<Value> = storage
            .db()
            .unwrap()
            .query(LIST_OPERATION_IDS_BY_STATE_QUERY)
            .bind(("state", "blocked"))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        let row = rows
            .iter()
            .find(|row| row["operation_id"] == "projected-reconciliation")
            .expect("nonterminal operation identity is projected");
        assert_eq!(row.as_object().unwrap().len(), 1);
        assert!(!serde_json::to_string(&rows).unwrap().contains(sentinel));

        let receipt = service
            .get("projected-reconciliation")
            .await
            .unwrap()
            .expect("operation receipt");
        assert_eq!(receipt.operation_id, "projected-reconciliation");
        let receipt_rows: Vec<Value> = storage
            .db()
            .unwrap()
            .query(GET_OPERATION_RECEIPT_QUERY)
            .bind(("id", "projected-reconciliation".to_owned()))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        let receipt_row = receipt_rows.first().expect("projected receipt row");
        assert!(!receipt_row.as_object().unwrap().contains_key("payload"));
        assert!(
            !serde_json::to_string(&receipt_rows)
                .unwrap()
                .contains(sentinel)
        );
    }

    #[tokio::test]
    async fn lagged_event_subscriber_backfills_every_durable_sequence() {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        let service = OperationService::start_with_capacities(
            storage as Arc<dyn MemoryStorage>,
            embedder,
            Duration::from_secs(10),
            16,
            4,
        );
        let payload = json!({
            "name": "event-stream",
            "description": "lag recovery fixture",
            "agent_id": null,
            "user_id": "test"
        });
        service
            .submit(OperationRequest {
                operation_id: "lagged-events".to_owned(),
                schema_version: OPERATION_SCHEMA_VERSION,
                kind: "create_task_stream".to_owned(),
                dependencies: vec!["missing-prerequisite".to_owned()],
                payload_hash: payload_hash(&payload).unwrap(),
                payload,
            })
            .await
            .unwrap();
        wait_for_service_state(&service, "lagged-events", OperationState::Blocked, false).await;

        let mut events = service.event_stream("lagged-events", 0).await.unwrap();
        for marker in 0..8 {
            service
                .transition(
                    "lagged-events",
                    OperationState::Blocked,
                    vec!["missing-prerequisite".to_owned()],
                    None,
                    None,
                    Some(json!({"marker":marker})),
                )
                .await
                .unwrap();
        }

        let final_sequence = service
            .get("lagged-events")
            .await
            .unwrap()
            .unwrap()
            .progress_seq;
        let mut sequences = Vec::new();
        while sequences.last().copied().unwrap_or(0) < final_sequence {
            sequences.push(events.next().await.unwrap().unwrap().sequence);
        }
        assert_eq!(sequences, (1..=final_sequence).collect::<Vec<_>>());
    }
}
