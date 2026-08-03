//! Durable, receipt-driven operation ingestion.
//!
//! The operation ledger is the acknowledgement boundary.  A caller-provided
//! id is persisted before any model inference starts, and every subsequent
//! action is recoverable from explicit database state.  Wall-clock duration is
//! never used to infer whether an operation succeeded.

use std::{convert::Infallible, sync::Arc};

use anyhow::{Context, Result};
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
    embeddings::EmbeddingPlanPart,
};
use surrealdb::types::{Datetime, RecordId};
use surrealdb_types::SurrealValue;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

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
    progress_seq: u64,
    created_at: Datetime,
    updated_at: Datetime,
}

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
    wake_tx: mpsc::Sender<String>,
    events_tx: broadcast::Sender<OperationEvent>,
}

#[derive(Debug)]
enum SubmitError {
    Invalid(anyhow::Error),
    Conflict(OperationReceipt),
    Storage(anyhow::Error),
}

impl OperationService {
    pub fn start(
        storage: Arc<dyn MemoryStorage>,
        embedding_service: Arc<dyn EmbeddingService>,
    ) -> Self {
        let (wake_tx, wake_rx) = mpsc::channel(256);
        let (events_tx, _) = broadcast::channel(1024);
        let service = Self {
            storage,
            embedding_service,
            wake_tx,
            events_tx,
        };
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

    async fn submit(
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
                return Err(SubmitError::Conflict(existing));
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
        let db = self
            .surreal()
            .map_err(SubmitError::Storage)?
            .db()
            .map_err(SubmitError::Storage)?;
        let response = db
            .query(
                "BEGIN TRANSACTION;\n\
                 CREATE type::record('memory_operation', $key) CONTENT $operation;\n\
                 CREATE type::record('memory_operation_event', $event_key) CONTENT $event;\n\
                 COMMIT TRANSACTION;",
            )
            .bind(("key", key))
            .bind(("operation", db_record))
            .bind(("event_key", event_key))
            .bind(("event", event))
            .await;

        if let Err(error) = response.and_then(|response| response.check()) {
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
                return Err(SubmitError::Conflict(existing));
            }
            return Err(SubmitError::Storage(error.into()));
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
        let db = self.surreal()?.db()?;
        let rows: Vec<DbOperation> = db
            .query("SELECT * FROM memory_operation WHERE operation_id = $id LIMIT 1")
            .bind(("id", operation_id.to_owned()))
            .await?
            .check()?
            .take(0)?;
        rows.into_iter().next().map(TryInto::try_into).transpose()
    }

    async fn get_db(&self, operation_id: &str) -> Result<Option<DbOperation>> {
        let db = self.surreal()?.db()?;
        let rows: Vec<DbOperation> = db
            .query("SELECT * FROM memory_operation WHERE operation_id = $id LIMIT 1")
            .bind(("id", operation_id.to_owned()))
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().next())
    }

    async fn list_nonterminal_ids(&self) -> Result<Vec<String>> {
        let db = self.surreal()?.db()?;
        let rows: Vec<DbOperation> = db
            .query("SELECT * FROM memory_operation WHERE state NOT IN ['committed', 'rejected'] ORDER BY operation_id ASC")
            .await?
            .check()?
            .take(0)?;
        Ok(rows.into_iter().map(|row| row.operation_id).collect())
    }

    async fn events_after(&self, operation_id: &str, sequence: u64) -> Result<Vec<OperationEvent>> {
        let db = self.surreal()?.db()?;
        let rows: Vec<DbOperationEvent> = db
            .query(
                "SELECT * FROM memory_operation_event WHERE operation_id = $id AND sequence > $sequence ORDER BY sequence ASC",
            )
            .bind(("id", operation_id.to_owned()))
            .bind(("sequence", sequence))
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
        let db = self.surreal()?.db()?;
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
        .bind(("event", event))
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
        let db = self.surreal()?.db()?;
        let parts: Vec<DbOperationPart> = db
            .query(
                "SELECT * FROM memory_operation_part WHERE operation_id = $id ORDER BY part_index ASC",
            )
            .bind(("id", operation_id.to_owned()))
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

        let db = self.surreal()?.db()?;
        for part in plan {
            let key = format!("{}-{:08}", record_key(operation_id), part.part_index);
            let row = DbOperationPart {
                id: None,
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
            };
            db.query("CREATE type::record('memory_operation_part', $key) CONTENT $part")
                .bind(("key", key))
                .bind(("part", row))
                .await?
                .check()?;
        }
        Ok(())
    }

    async fn mark_part_indexed(
        &self,
        operation_id: &str,
        part_index: u64,
        embedding: Vec<f32>,
    ) -> Result<()> {
        let db = self.surreal()?.db()?;
        db.query(
            "UPDATE memory_operation_part SET state = 'indexed', embedding = $embedding, updated_at = $now WHERE operation_id = $id AND part_index = $index",
        )
        .bind(("embedding", embedding))
        .bind(("now", Datetime::default()))
        .bind(("id", operation_id.to_owned()))
        .bind(("index", part_index))
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

    async fn run(self, mut wake_rx: mpsc::Receiver<String>) {
        match self.list_nonterminal_ids().await {
            Ok(ids) => {
                for id in ids {
                    if self.wake_tx.send(id).await.is_err() {
                        return;
                    }
                }
            }
            Err(error) => tracing::error!(%error, "operation startup reconciliation failed"),
        }

        while let Some(operation_id) = wake_rx.recv().await {
            if let Err(error) = self.process(&operation_id).await {
                tracing::error!(%operation_id, %error, "operation processing paused");
                self.record_processing_error(&operation_id, &error).await;
            }
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

        // A committed prerequisite is an explicit event that makes blocked
        // dependents runnable; no timer or polling loop is involved.
        for id in self.list_nonterminal_ids().await? {
            if id != operation_id {
                let _ = self.wake_tx.send(id).await;
            }
        }
        Ok(())
    }

    async fn process_add_memory(
        &self,
        operation: &DbOperation,
        mut state: OperationState,
    ) -> Result<()> {
        let request: AddMemoryRequest = serde_json::from_value(operation.payload.clone())
            .context("invalid add_memory payload")?;
        let plan = self.embedding_service.plan(&request.content).await?;
        if plan.is_empty() {
            anyhow::bail!("embedding planner returned no parts");
        }
        self.persist_plan(&operation.operation_id, &plan).await?;

        if state == OperationState::Validated {
            self.transition(
                &operation.operation_id,
                OperationState::Planned,
                Vec::new(),
                None,
                None,
                Some(json!({
                    "parts": plan.len(),
                    "token_count": plan.iter().map(|part| part.token_end).max().unwrap_or(0),
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

        let mut stored_parts = self.operation_parts(&operation.operation_id).await?;
        if state == OperationState::Processing {
            for part in &mut stored_parts {
                if part.state == "indexed" && part.embedding.is_some() {
                    continue;
                }
                let embedding = self.embedding_service.embed(&part.content).await?;
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
        let token_count = stored_parts
            .iter()
            .map(|part| part.token_end)
            .max()
            .unwrap_or(0)
            .try_into()
            .context("logical memory token count exceeds u32")?;
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

    // Subscribe before reading history. Any transition racing with the query
    // is either present in the snapshot or delivered by the live receiver;
    // sequence filtering removes the harmless overlap.
    let live = state.operations.events_tx.subscribe();
    let history = state
        .operations
        .events_after(&id, query.after)
        .await
        .map_err(crate::api::api_error)?;
    let history_max = history
        .last()
        .map(|event| event.sequence)
        .unwrap_or(query.after);
    let history_stream = tokio_stream::iter(history.into_iter());
    let id_for_filter = id.clone();
    let live_stream = BroadcastStream::new(live).filter_map(move |item| match item {
        Ok(event) if event.operation_id == id_for_filter && event.sequence > history_max => {
            Some(event)
        }
        _ => None,
    });
    let stream = history_stream.chain(live_stream).map(|event| {
        let id = event.sequence.to_string();
        let data = serde_json::to_string(&event).expect("operation event serializes");
        Ok(Event::default().event("operation_state").id(id).data(data))
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

    async fn test_router() -> Router {
        let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
        let storage: Arc<dyn MemoryStorage> = Arc::new(
            SurrealStorage::new_mem(Arc::clone(&embedder))
                .await
                .expect("in-memory SurrealStorage"),
        );
        crate::api::build_router(storage, embedder)
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

    async fn wait_for_state(router: Router, id: &str, expected: OperationState) {
        // Iteration-bounded cooperative scheduling: runtime correctness does
        // not use elapsed time, and this test never sleeps to guess machine
        // performance.
        let mut last = None;
        for _ in 0..10_000 {
            let receipt = get_receipt(router.clone(), id).await;
            if receipt.state == expected {
                return;
            }
            last = Some(receipt);
            tokio::task::yield_now().await;
        }
        panic!("operation {id} did not reach {expected:?}; last receipt: {last:?}");
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
        let router = test_router().await;
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
        wait_for_state(router.clone(), "same-id", OperationState::Committed).await;

        let receipt = get_receipt(router, "same-id").await;
        assert_eq!(receipt.operation_id, "same-id");
        assert_eq!(receipt.progress_seq, 6);
    }

    #[tokio::test]
    async fn reusing_operation_id_with_another_hash_is_conflict() {
        let router = test_router().await;
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
    async fn explicit_dependencies_unblock_in_topological_order() {
        let router = test_router().await;
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
        wait_for_state(router.clone(), "op-stream", OperationState::Committed).await;
        wait_for_state(router.clone(), "op-step", OperationState::Committed).await;
        wait_for_state(router, "op-complete", OperationState::Committed).await;
    }
}
