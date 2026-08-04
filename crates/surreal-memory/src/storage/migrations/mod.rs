//! Migration runner for surreal-memory schema evolution.
//!
//! Migrations are additive — never destructive. The runner reads the highest
//! applied version from `schema_version` and runs only pending migrations.
//! Safe to call at every startup.
//!
//! KEY DESIGN DECISIONS:
//! - mindmap table is SCHEMALESS: node/edge objects carry arbitrary fields
//!   (color, metadata, node_type, etc.) which SCHEMAFULL rejects without
//!   explicit sub-field definitions for every array element field.
//! - task_stream includes all struct fields upfront to avoid schema mismatches.
//! - memory.metadata is FLEXIBLE to allow arbitrary JSON metadata objects.

use crate::{
    memory::{MemoryScope, MemoryType},
    mindmap::MapType,
    task_stream::TaskStreamStatus,
};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{future::Future, pin::Pin};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::types::{Datetime, RecordId, RecordIdKey};
use surrealdb_types::SurrealValue;

type MigrationFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
type RepairFn = for<'a> fn(&'a Surreal<Any>) -> MigrationFuture<'a>;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
    pub repair: Option<RepairFn>,
    pub checksum_source: &'static str,
}

impl Migration {
    const fn sql(version: u32, name: &'static str, sql: &'static str) -> Self {
        Self {
            version,
            name,
            sql,
            repair: None,
            checksum_source: sql,
        }
    }

    const fn repair(
        version: u32,
        name: &'static str,
        repair: RepairFn,
        checksum_source: &'static str,
    ) -> Self {
        Self {
            version,
            name,
            sql: "",
            repair: Some(repair),
            checksum_source,
        }
    }
}

static MIGRATIONS: &[Migration] = &[
    Migration::sql(1, "initial_entity_relation_schema", MIGRATION_V1_SQL),
    Migration::sql(2, "scoped_memory_table", MIGRATION_V2_SQL),
    Migration::sql(3, "task_stream_table", MIGRATION_V3_SQL),
    Migration::sql(4, "memory_history_table", MIGRATION_V4_SQL),
    Migration::sql(5, "hnsw_vector_indexes", MIGRATION_V5_SQL),
    Migration::sql(6, "mindmap_table_and_fulltext_indexes", MIGRATION_V6_SQL),
    Migration::sql(7, "task_stream_auto_summarization_fields", MIGRATION_V7_SQL),
    Migration::sql(8, "memory_metadata_flexible", MIGRATION_V8_SQL),
    Migration::sql(9, "mindmap_nodes_edges_flexible", MIGRATION_V9_SQL),
    Migration::sql(
        10,
        "mindmap_nodes_edges_flexible_overwrite",
        MIGRATION_V10_SQL,
    ),
    Migration::sql(11, "mindmap_nodes_edges_remove_redefine", MIGRATION_V11_SQL),
    Migration::sql(
        12,
        "mindmap_node_element_fields_flexible",
        MIGRATION_V12_SQL,
    ),
    Migration::sql(13, "mindmap_edge_nested_fields", MIGRATION_V13_SQL),
    Migration::repair(
        14,
        "legacy_enum_string_normalization",
        repair_legacy_enum_data_migration,
        "legacy_enum_string_normalization_v14",
    ),
    Migration::sql(15, "enum_fields_as_strings", MIGRATION_V15_SQL),
    Migration::sql(16, "palace_drawers_table", MIGRATION_V16_SQL),
    Migration::sql(17, "dynamic_embedding_index_metadata", MIGRATION_V17_SQL),
    Migration::sql(18, "task_stream_scope_unique_index", MIGRATION_V18_SQL),
    Migration::sql(19, "task_step_table", MIGRATION_V19_SQL),
    Migration::sql(20, "durable_operation_ledger", MIGRATION_V20_SQL),
    Migration::sql(21, "embedding_executor_journal", MIGRATION_V21_SQL),
];

// ── v1: Baseline entity + relation schema ─────────────────────────────────────

const MIGRATION_V1_SQL: &str = "
DEFINE TABLE IF NOT EXISTS entity SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON entity TYPE string;
DEFINE FIELD IF NOT EXISTS entity_type ON entity TYPE string;
DEFINE FIELD IF NOT EXISTS observations ON entity TYPE array<string>;
DEFINE FIELD IF NOT EXISTS observations.* ON entity TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON entity TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON entity TYPE datetime;
DEFINE FIELD IF NOT EXISTS embedding ON entity TYPE option<array<float>>;
DEFINE FIELD IF NOT EXISTS embedding.* ON entity TYPE float;
DEFINE INDEX IF NOT EXISTS entity_name ON entity FIELDS name UNIQUE;

DEFINE TABLE IF NOT EXISTS relation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS from ON relation TYPE string;
DEFINE FIELD IF NOT EXISTS to ON relation TYPE string;
DEFINE FIELD IF NOT EXISTS relation_type ON relation TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON relation TYPE datetime;
DEFINE INDEX IF NOT EXISTS relation_unique ON relation FIELDS from, to, relation_type UNIQUE;

DEFINE TABLE IF NOT EXISTS schema_version SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version ON schema_version TYPE int;
DEFINE FIELD IF NOT EXISTS migration_name ON schema_version TYPE string;
DEFINE FIELD IF NOT EXISTS applied_at ON schema_version TYPE datetime;
DEFINE FIELD IF NOT EXISTS checksum ON schema_version TYPE string;
";

// ── v2: Scoped Memory ─────────────────────────────────────────────────────────

const MIGRATION_V2_SQL: &str = "
DEFINE TABLE IF NOT EXISTS memory SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS content ON memory TYPE string;
DEFINE FIELD IF NOT EXISTS embedding ON memory TYPE option<array<float>>;
DEFINE FIELD IF NOT EXISTS embedding.* ON memory TYPE float;
DEFINE FIELD IF NOT EXISTS scope ON memory TYPE any DEFAULT 'global';
DEFINE FIELD IF NOT EXISTS memory_type ON memory TYPE any DEFAULT 'semantic';
DEFINE FIELD IF NOT EXISTS user_id ON memory TYPE option<string>;
DEFINE FIELD IF NOT EXISTS session_id ON memory TYPE option<string>;
DEFINE FIELD IF NOT EXISTS agent_id ON memory TYPE option<string>;
DEFINE FIELD IF NOT EXISTS task_stream_id ON memory TYPE option<record<task_stream>>;
DEFINE FIELD IF NOT EXISTS categories ON memory TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS categories.* ON memory TYPE string;
DEFINE FIELD IF NOT EXISTS metadata ON memory TYPE option<object>;
DEFINE FIELD IF NOT EXISTS token_count ON memory TYPE option<int>;
DEFINE FIELD IF NOT EXISTS importance ON memory TYPE float DEFAULT 0.5;
DEFINE FIELD IF NOT EXISTS access_count ON memory TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_accessed_at ON memory TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS valid_until ON memory TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS version ON memory TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS created_at ON memory TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON memory TYPE datetime;

DEFINE INDEX IF NOT EXISTS memory_user ON memory FIELDS user_id;
DEFINE INDEX IF NOT EXISTS memory_agent ON memory FIELDS agent_id;
DEFINE INDEX IF NOT EXISTS memory_session ON memory FIELDS session_id;
DEFINE INDEX IF NOT EXISTS memory_scope ON memory FIELDS scope, user_id, agent_id, session_id;
";

// ── v3: TaskStream — all struct fields including auto-summarization ────────────
// Includes auto_summarize, summary_count, model_id that the TaskStream struct
// requires. Defining them here avoids schema mismatch on insert.

const MIGRATION_V3_SQL: &str = "
DEFINE TABLE IF NOT EXISTS task_stream SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON task_stream TYPE string;
DEFINE FIELD IF NOT EXISTS description ON task_stream TYPE option<string>;
DEFINE FIELD IF NOT EXISTS agent_id ON task_stream TYPE option<string>;
DEFINE FIELD IF NOT EXISTS user_id ON task_stream TYPE option<string>;
DEFINE FIELD IF NOT EXISTS status ON task_stream TYPE any DEFAULT 'active';
DEFINE FIELD IF NOT EXISTS total_tokens ON task_stream TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS auto_summarize ON task_stream TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS summary_count ON task_stream TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS model_id ON task_stream TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON task_stream TYPE datetime;
DEFINE FIELD IF NOT EXISTS last_active ON task_stream TYPE datetime;
DEFINE INDEX IF NOT EXISTS task_stream_name ON task_stream FIELDS name UNIQUE;
";

// ── v4: MemoryHistory audit log ───────────────────────────────────────────────

const MIGRATION_V4_SQL: &str = "
DEFINE TABLE IF NOT EXISTS memory_history SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON memory_history TYPE record<memory>;
DEFINE FIELD IF NOT EXISTS version ON memory_history TYPE int;
DEFINE FIELD IF NOT EXISTS old_content ON memory_history TYPE option<string>;
DEFINE FIELD IF NOT EXISTS new_content ON memory_history TYPE string;
DEFINE FIELD IF NOT EXISTS changed_at ON memory_history TYPE datetime;
DEFINE FIELD IF NOT EXISTS change_type ON memory_history TYPE string;
DEFINE INDEX IF NOT EXISTS memory_history_by_memory ON memory_history FIELDS memory_id;
";

// ── v5: HNSW vector indexes ───────────────────────────────────────────────────

const MIGRATION_V5_SQL: &str = "
DEFINE INDEX IF NOT EXISTS entity_embedding_hnsw
  ON entity FIELDS embedding HNSW DIMENSION 1536 DIST COSINE TYPE F32;
DEFINE INDEX IF NOT EXISTS memory_embedding_hnsw
  ON memory FIELDS embedding HNSW DIMENSION 1536 DIST COSINE TYPE F32;
";

// ── v6: Mindmap table — SCHEMALESS ────────────────────────────────────────────
// CRITICAL: mindmap MUST be SCHEMALESS (not SCHEMAFULL).
// MindMapNode carries optional fields (color, metadata, node_type, parent_id)
// that SurrealDB SCHEMAFULL mode rejects unless every array element sub-field
// is explicitly defined — which breaks with arbitrary JSON in metadata.
// SCHEMALESS gives full flexibility for node/edge objects while still
// allowing indexed lookups on the top-level name and agent_id fields.

const MIGRATION_V6_SQL: &str = "
DEFINE TABLE IF NOT EXISTS mindmap SCHEMALESS;
DEFINE INDEX IF NOT EXISTS mindmap_name ON mindmap FIELDS name, user_id UNIQUE;
DEFINE INDEX IF NOT EXISTS mindmap_agent ON mindmap FIELDS agent_id;
";

// ── v7: TaskStream auto-summarization fields ─────────────────────────────────
//
// The TaskStream struct added `auto_summarize`, `summary_count`, and `model_id`
// for model-aware rolling summarization, but the v3 schema didn't include them.

const MIGRATION_V7_SQL: &str = "
DEFINE FIELD IF NOT EXISTS auto_summarize ON task_stream TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS summary_count ON task_stream TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS model_id ON task_stream TYPE option<string>;
";

// ── v8: Memory metadata FLEXIBLE ─────────────────────────────────────────────
//
// The `metadata` field was `option<object>` which rejects arbitrary nested JSON
// in SCHEMAFULL mode. Redefine as FLEXIBLE so callers can store structured
// metadata (rationale, alternatives, file lists, etc.) without schema conflicts.

const MIGRATION_V8_SQL: &str = "
DEFINE FIELD IF NOT EXISTS metadata ON memory TYPE option<object> FLEXIBLE;
";

// ── v9: Mindmap nodes/edges FLEXIBLE ─────────────────────────────────────────
//
// The `nodes` and `edges` fields were defined as `array<object>` (strict) which
// causes SurrealDB SCHEMAFULL validation to reject nested objects containing
// arbitrary fields (e.g. `metadata: Option<serde_json::Value>` in MindMapNode).
// For server-mode compatibility we keep the table SCHEMALESS and only type the
// top-level arrays here; nested fields are added explicitly in later migrations.

// Use OVERWRITE (not IF NOT EXISTS) so we force-update existing strict field
// definitions from earlier versions.
const MIGRATION_V9_SQL: &str = "
DEFINE FIELD OVERWRITE nodes ON mindmap TYPE array<object> DEFAULT [];
DEFINE FIELD OVERWRITE edges ON mindmap TYPE array<object> DEFAULT [];
";

// ── v10: Force overwrite mindmap nodes/edges again ───────────────────────────
//
// Keep this as a no-op overwrite for upgraded databases that already saw v9.

const MIGRATION_V10_SQL: &str = "
DEFINE FIELD OVERWRITE nodes ON mindmap TYPE array<object> DEFAULT [];
DEFINE FIELD OVERWRITE edges ON mindmap TYPE array<object> DEFAULT [];
";

// ── v11: Remove and redefine mindmap nodes/edges ─────────────────────────────
//
// OVERWRITE alone does not clear existing sub-field constraints in SurrealDB 3.x.
// The v6 schema defined nodes/edges as strict array<object> which rejects any
// object field not explicitly known to the schema (e.g. nodes[0].color).
// Remove the fields entirely to purge sub-field metadata, then redefine the
// typed arrays cleanly before adding explicit nested fields.

const MIGRATION_V11_SQL: &str = "
REMOVE FIELD IF EXISTS nodes ON mindmap;
REMOVE FIELD IF EXISTS edges ON mindmap;
DEFINE FIELD nodes ON mindmap TYPE array<object> DEFAULT [];
DEFINE FIELD edges ON mindmap TYPE array<object> DEFAULT [];
";

// ── v12: Explicit nested schema for mindmap node objects ─────────────────────
//
// Server-mode SurrealDB validates array element shapes once the top-level field
// exists, even on a SCHEMALESS table. Define the persisted node fields
// explicitly so create/update calls match the Rust structs.

const MIGRATION_V12_SQL: &str = "
DEFINE FIELD IF NOT EXISTS nodes.* ON mindmap TYPE object;
DEFINE FIELD IF NOT EXISTS nodes.*.id ON mindmap TYPE string;
DEFINE FIELD IF NOT EXISTS nodes.*.label ON mindmap TYPE string;
DEFINE FIELD IF NOT EXISTS nodes.*.parent_id ON mindmap TYPE option<string>;
DEFINE FIELD IF NOT EXISTS nodes.*.node_type ON mindmap TYPE option<string>;
DEFINE FIELD IF NOT EXISTS nodes.*.color ON mindmap TYPE option<string>;
DEFINE FIELD IF NOT EXISTS nodes.*.metadata ON mindmap TYPE option<object>;
DEFINE FIELD IF NOT EXISTS edges.* ON mindmap TYPE object;
";

// ── v13: Explicit nested schema for mindmap edge objects ─────────────────────
//
// Server-mode SCHEMAFULL validation can still reject `edges[*].directed` when
// only the wildcard element object is marked FLEXIBLE. Define the concrete
// nested edge fields explicitly so fresh namespaces and upgraded deployments
// accept the serialized `MindMapEdge` shape.

const MIGRATION_V13_SQL: &str = "
DEFINE FIELD IF NOT EXISTS edges.*.from_id ON mindmap TYPE string;
DEFINE FIELD IF NOT EXISTS edges.*.to_id ON mindmap TYPE string;
DEFINE FIELD IF NOT EXISTS edges.*.label ON mindmap TYPE option<string>;
DEFINE FIELD IF NOT EXISTS edges.*.directed ON mindmap TYPE bool;
";

// ── v15: Persist enum-like fields as canonical strings ──────────────────────

const MIGRATION_V15_SQL: &str = "
DEFINE FIELD OVERWRITE scope ON memory TYPE string DEFAULT 'global';
DEFINE FIELD OVERWRITE memory_type ON memory TYPE string DEFAULT 'semantic';
DEFINE FIELD OVERWRITE status ON task_stream TYPE string DEFAULT 'active';
DEFINE FIELD OVERWRITE map_type ON mindmap TYPE string DEFAULT 'radial';
";

// ── v16: MemPalace drawers table ────────────────────────────────────────────

const MIGRATION_V16_SQL: &str = "
DEFINE TABLE IF NOT EXISTS drawers SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS content      ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS wing         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS room         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS hall         ON drawers TYPE string;
DEFINE FIELD IF NOT EXISTS source_file  ON drawers TYPE option<string>;
DEFINE FIELD IF NOT EXISTS date         ON drawers TYPE option<string>;
DEFINE FIELD IF NOT EXISTS importance   ON drawers TYPE float DEFAULT 1.0;
DEFINE FIELD IF NOT EXISTS embedding    ON drawers TYPE option<array<float>>;
DEFINE FIELD IF NOT EXISTS embedding.*  ON drawers TYPE float;
DEFINE FIELD IF NOT EXISTS content_hash ON drawers TYPE option<string>;

DEFINE INDEX IF NOT EXISTS drawer_embedding_idx
  ON drawers FIELDS embedding
  HNSW DIMENSION 384 DIST COSINE TYPE F32;

-- Full-text BM25 index omitted: SurrealDB 3.x SEARCH syntax requires further investigation.
-- HNSW vector search is the primary retrieval path; keyword search can be added later.

DEFINE INDEX IF NOT EXISTS drawer_hash_idx
  ON drawers FIELDS content_hash;

DEFINE INDEX IF NOT EXISTS drawer_wing_idx ON drawers FIELDS wing;
DEFINE INDEX IF NOT EXISTS drawer_room_idx ON drawers FIELDS room;
";

// ── v17: Dynamic embedding index metadata ───────────────────────────────────
//
// v5 hardcoded HNSW indexes to OpenAI's 1536 dimensions. Local models such as
// BAAI/bge-small-en-v1.5 emit 384-dimensional vectors, so startup now recreates
// the vector indexes from the active embedding provider dimensions.

const MIGRATION_V17_SQL: &str = "
REMOVE INDEX IF EXISTS entity_embedding_hnsw ON entity;
REMOVE INDEX IF EXISTS memory_embedding_hnsw ON memory;

DEFINE TABLE IF NOT EXISTS schema_metadata SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON schema_metadata TYPE string;
DEFINE FIELD IF NOT EXISTS int_value ON schema_metadata TYPE option<int>;
DEFINE FIELD IF NOT EXISTS updated_at ON schema_metadata TYPE datetime;
DEFINE INDEX IF NOT EXISTS schema_metadata_name ON schema_metadata FIELDS name UNIQUE;
";

// ── v18: Scope-qualified TaskStream name uniqueness ─────────────────────────
//
// v3 defined `task_stream_name` as a GLOBAL unique index on `name`, which
// blocks two agents (or two users) from each owning a stream with the same
// name (e.g. both having a "build" stream). Replace it with a composite
// unique index over (agent_id, user_id, name) so uniqueness is scoped.
//
// `DbTaskStream::from` now stores absent agent_id/user_id as the empty string
// "". Rows created before this migration stored them as NONE, so the backfill
// statements below normalize those legacy rows to "" FIRST — otherwise the
// composite unique index would see a mixed NONE/"" key space and enforce
// uniqueness inconsistently. Order matters: backfill, then REMOVE, then DEFINE.
//
// After the backfill the index key space is uniform: two rows with identical
// (agent_id, user_id, name) collide, preserving "one stream per name per
// scope" semantics.

const MIGRATION_V18_SQL: &str = "
UPDATE task_stream SET agent_id = '' WHERE agent_id IS NONE;
UPDATE task_stream SET user_id = '' WHERE user_id IS NONE;
REMOVE INDEX IF EXISTS task_stream_name ON task_stream;
DEFINE INDEX IF NOT EXISTS task_stream_scope_name
  ON task_stream FIELDS agent_id, user_id, name UNIQUE;
";

// ── v19: TaskStep — first-class ordered, status-tracked steps ───────────────
//
// Introduces the `task_step` table backing the `TaskStep` struct. SCHEMAFULL,
// so every persisted field MUST have a DEFINE FIELD or inserts fail at runtime.
// `status` is stored as a string (enums-as-strings standard from v15).
// `idempotency_key` carries a UNIQUE index so step creation and completion are
// safe to replay. The composite (task_stream_id, ordinal) index backs ordered
// retrieval and current-step lookups.

const MIGRATION_V19_SQL: &str = "
DEFINE TABLE IF NOT EXISTS task_step SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS task_stream_id ON task_step TYPE option<record<task_stream>>;
DEFINE FIELD IF NOT EXISTS ordinal ON task_step TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS name ON task_step TYPE string;
DEFINE FIELD IF NOT EXISTS description ON task_step TYPE option<string>;
DEFINE FIELD IF NOT EXISTS status ON task_step TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS idempotency_key ON task_step TYPE string;
DEFINE FIELD IF NOT EXISTS result ON task_step TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error ON task_step TYPE option<string>;
DEFINE FIELD IF NOT EXISTS started_at ON task_step TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS completed_at ON task_step TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS created_at ON task_step TYPE datetime;
DEFINE INDEX IF NOT EXISTS task_step_stream_ordinal ON task_step FIELDS task_stream_id, ordinal;
DEFINE INDEX IF NOT EXISTS task_step_idempotency_key ON task_step FIELDS idempotency_key UNIQUE;
";

// ── v20: Durable operation ledger ───────────────────────────────────────────
//
// The operation id is supplied by the caller and is the transport-level
// idempotency boundary.  An accepted row exists before any model inference or
// semantic lookup starts.  The payload is deliberately retained verbatim so a
// coordinator can resume after a crash without asking the caller to resend it.
// State changes are also copied to an append-only event table.  Timestamps are
// audit data only; no transition is inferred from elapsed wall-clock time.

const MIGRATION_V20_SQL: &str = "
DEFINE TABLE IF NOT EXISTS memory_operation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS operation_id ON memory_operation TYPE string;
DEFINE FIELD IF NOT EXISTS schema_version ON memory_operation TYPE int;
DEFINE FIELD IF NOT EXISTS kind ON memory_operation TYPE string;
DEFINE FIELD IF NOT EXISTS dependencies ON memory_operation TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS dependencies.* ON memory_operation TYPE string;
DEFINE FIELD IF NOT EXISTS payload_hash ON memory_operation TYPE string;
DEFINE FIELD IF NOT EXISTS payload ON memory_operation TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS state ON memory_operation TYPE string;
DEFINE FIELD IF NOT EXISTS blocked_by ON memory_operation TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS blocked_by.* ON memory_operation TYPE string;
DEFINE FIELD IF NOT EXISTS result ON memory_operation TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS error ON memory_operation TYPE option<string>;
DEFINE FIELD IF NOT EXISTS executor_generation ON memory_operation TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS progress_seq ON memory_operation TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON memory_operation TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON memory_operation TYPE datetime;
DEFINE INDEX IF NOT EXISTS memory_operation_id ON memory_operation FIELDS operation_id UNIQUE;
DEFINE INDEX IF NOT EXISTS memory_operation_state ON memory_operation FIELDS state;

DEFINE TABLE IF NOT EXISTS memory_operation_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS operation_id ON memory_operation_event TYPE string;
DEFINE FIELD IF NOT EXISTS sequence ON memory_operation_event TYPE int;
DEFINE FIELD IF NOT EXISTS from_state ON memory_operation_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS to_state ON memory_operation_event TYPE string;
DEFINE FIELD IF NOT EXISTS detail ON memory_operation_event TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS executor_generation ON memory_operation_event TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS occurred_at ON memory_operation_event TYPE datetime;
DEFINE INDEX IF NOT EXISTS memory_operation_event_sequence
  ON memory_operation_event FIELDS operation_id, sequence UNIQUE;

DEFINE TABLE IF NOT EXISTS memory_operation_part SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS operation_id ON memory_operation_part TYPE string;
DEFINE FIELD IF NOT EXISTS part_index ON memory_operation_part TYPE int;
DEFINE FIELD IF NOT EXISTS token_start ON memory_operation_part TYPE int;
DEFINE FIELD IF NOT EXISTS token_end ON memory_operation_part TYPE int;
DEFINE FIELD IF NOT EXISTS token_count ON memory_operation_part TYPE int;
DEFINE FIELD IF NOT EXISTS token_hash ON memory_operation_part TYPE string;
DEFINE FIELD IF NOT EXISTS content ON memory_operation_part TYPE string;
DEFINE FIELD IF NOT EXISTS state ON memory_operation_part TYPE string;
DEFINE FIELD IF NOT EXISTS embedding ON memory_operation_part TYPE option<array<float>>;
DEFINE FIELD IF NOT EXISTS embedding.* ON memory_operation_part TYPE float;
DEFINE FIELD IF NOT EXISTS updated_at ON memory_operation_part TYPE datetime;
DEFINE INDEX IF NOT EXISTS memory_operation_part_identity
  ON memory_operation_part FIELDS operation_id, part_index UNIQUE;
";

// ── v21: Supervised embedding executor journal ──────────────────────────────

const MIGRATION_V21_SQL: &str = "
DEFINE FIELD IF NOT EXISTS executor_progress_seq ON memory_operation TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS executor_exit_count ON memory_operation TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS executor_last_exit ON memory_operation TYPE option<string>;
DEFINE FIELD IF NOT EXISTS executor_error ON memory_operation TYPE option<string>;

DEFINE TABLE IF NOT EXISTS memory_executor_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS operation_id ON memory_executor_event TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON memory_executor_event TYPE int;
DEFINE FIELD IF NOT EXISTS progress_seq ON memory_executor_event TYPE int;
DEFINE FIELD IF NOT EXISTS kind ON memory_executor_event TYPE string;
DEFINE FIELD IF NOT EXISTS message ON memory_executor_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS occurred_at ON memory_executor_event TYPE datetime;
DEFINE INDEX IF NOT EXISTS memory_executor_event_identity
  ON memory_executor_event FIELDS operation_id, generation, progress_seq UNIQUE;
";

#[derive(Debug, Clone)]
pub struct RepairChange {
    pub table: &'static str,
    pub field: &'static str,
    pub record: RecordId,
    pub record_id: String,
    pub raw_value: serde_json::Value,
    pub normalized_value: String,
}

#[derive(Debug, Clone)]
pub struct RepairIssue {
    pub table: &'static str,
    pub field: &'static str,
    pub record_id: String,
    pub raw_value: serde_json::Value,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct RepairReport {
    pub scanned_records: usize,
    pub changes: Vec<RepairChange>,
    pub issues: Vec<RepairIssue>,
    pub repairs_applied: usize,
}

impl RepairReport {
    pub fn planned_repairs(&self) -> usize {
        self.changes.len()
    }

    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct RawMemoryEnumRecord {
    id: RecordId,
    #[serde(default)]
    scope: serde_json::Value,
    #[serde(default)]
    memory_type: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct RawTaskStreamEnumRecord {
    id: RecordId,
    #[serde(default)]
    status: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct RawMindMapEnumRecord {
    id: RecordId,
    #[serde(default)]
    map_type: serde_json::Value,
}

pub async fn inspect_legacy_enum_data(db: &Surreal<Any>) -> Result<RepairReport> {
    let mut report = RepairReport::default();

    let memory_rows: Vec<RawMemoryEnumRecord> = db
        .query("SELECT id, scope, memory_type FROM memory")
        .await
        .context("Failed to inspect memory enum fields")?
        .take(0)
        .unwrap_or_default();
    report.scanned_records += memory_rows.len();
    for row in memory_rows {
        inspect_enum_field(
            &mut report,
            "memory",
            "scope",
            row.id.clone(),
            row.scope,
            |raw| MemoryScope::parse_str(raw).map(|value| value.as_str().to_string()),
        );
        inspect_enum_field(
            &mut report,
            "memory",
            "memory_type",
            row.id,
            row.memory_type,
            |raw| MemoryType::parse_str(raw).map(|value| value.as_str().to_string()),
        );
    }

    let task_stream_rows: Vec<RawTaskStreamEnumRecord> = db
        .query("SELECT id, status FROM task_stream")
        .await
        .context("Failed to inspect task_stream enum fields")?
        .take(0)
        .unwrap_or_default();
    report.scanned_records += task_stream_rows.len();
    for row in task_stream_rows {
        inspect_enum_field(
            &mut report,
            "task_stream",
            "status",
            row.id,
            row.status,
            |raw| TaskStreamStatus::parse_str(raw).map(|value| value.as_str().to_string()),
        );
    }

    let mindmap_rows: Vec<RawMindMapEnumRecord> = db
        .query("SELECT id, map_type FROM mindmap")
        .await
        .context("Failed to inspect mindmap enum fields")?
        .take(0)
        .unwrap_or_default();
    report.scanned_records += mindmap_rows.len();
    for row in mindmap_rows {
        inspect_enum_field(
            &mut report,
            "mindmap",
            "map_type",
            row.id,
            row.map_type,
            |raw| MapType::parse_str(raw).map(|value| value.as_str().to_string()),
        );
    }

    Ok(report)
}

pub async fn repair_legacy_enum_data(db: &Surreal<Any>) -> Result<RepairReport> {
    let mut report = inspect_legacy_enum_data(db).await?;
    if report.has_issues() {
        anyhow::bail!(format_repair_issues(&report.issues));
    }

    for change in &report.changes {
        apply_repair_change(db, change).await.with_context(|| {
            format!(
                "Failed to normalize {}.{} for {}",
                change.table, change.field, change.record_id
            )
        })?;
        report.repairs_applied += 1;
    }

    Ok(report)
}

fn repair_legacy_enum_data_migration(db: &Surreal<Any>) -> MigrationFuture<'_> {
    Box::pin(async move { repair_legacy_enum_data(db).await.map(|_| ()) })
}

fn inspect_enum_field(
    report: &mut RepairReport,
    table: &'static str,
    field: &'static str,
    record: RecordId,
    raw_value: serde_json::Value,
    canonicalize: impl Fn(&str) -> std::result::Result<String, String>,
) {
    match normalize_enum_value(table, field, &record, &raw_value, canonicalize) {
        Ok(Some(normalized_value)) => report.changes.push(RepairChange {
            table,
            field,
            record_id: record_id_to_string(&record),
            record,
            raw_value,
            normalized_value,
        }),
        Ok(None) => {}
        Err(issue) => report.issues.push(issue),
    }
}

fn normalize_enum_value(
    table: &'static str,
    field: &'static str,
    record: &RecordId,
    raw_value: &serde_json::Value,
    canonicalize: impl Fn(&str) -> std::result::Result<String, String>,
) -> std::result::Result<Option<String>, RepairIssue> {
    let record_id = record_id_to_string(record);

    let candidate = match raw_value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Object(map) if map.len() == 1 => map
            .keys()
            .next()
            .map(|value| value.trim().to_string())
            .unwrap_or_default(),
        other => {
            return Err(RepairIssue {
                table,
                field,
                record_id,
                raw_value: other.clone(),
                message: format!(
                    "unsupported legacy enum shape for {}.{}: expected string or single-key object",
                    table, field
                ),
            });
        }
    };

    let canonical_input = candidate.to_lowercase();
    let normalized_value = canonicalize(&canonical_input).map_err(|message| RepairIssue {
        table,
        field,
        record_id,
        raw_value: raw_value.clone(),
        message,
    })?;

    if *raw_value == serde_json::Value::String(normalized_value.clone()) {
        Ok(None)
    } else {
        Ok(Some(normalized_value))
    }
}

async fn apply_repair_change(db: &Surreal<Any>, change: &RepairChange) -> Result<()> {
    let sql = match (change.table, change.field) {
        ("memory", "scope") => "UPDATE $record SET scope = $value",
        ("memory", "memory_type") => "UPDATE $record SET memory_type = $value",
        ("task_stream", "status") => "UPDATE $record SET status = $value",
        ("mindmap", "map_type") => "UPDATE $record SET map_type = $value",
        _ => anyhow::bail!(
            "Unsupported repair target {}.{}",
            change.table,
            change.field
        ),
    };

    let response = db
        .query(sql)
        .bind(("record", change.record.clone()))
        .bind(("value", change.normalized_value.clone()))
        .await?;
    response.check()?;
    Ok(())
}

fn record_id_to_string(id: &RecordId) -> String {
    let key = match &id.key {
        RecordIdKey::String(value) => value.clone(),
        RecordIdKey::Number(value) => value.to_string(),
        RecordIdKey::Uuid(value) => value.to_string(),
        other => format!("{other:?}"),
    };
    format!("{}:{}", id.table.as_str(), key)
}

fn format_repair_issues(issues: &[RepairIssue]) -> String {
    let details = issues
        .iter()
        .map(|issue| {
            format!(
                "{}.{} {} raw={} error={}",
                issue.table, issue.field, issue.record_id, issue.raw_value, issue.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("Legacy enum normalization failed: {}", details)
}

// ── Runner ────────────────────────────────────────────────────────────────────

pub async fn run_migrations(db: &Surreal<Any>) -> Result<()> {
    let current_version = get_current_version(db).await?;
    tracing::info!("Current schema version: v{}", current_version);

    for migration in MIGRATIONS.iter().filter(|m| m.version > current_version) {
        apply_migration(db, migration).await.with_context(|| {
            format!(
                "Failed to apply migration v{}: {}",
                migration.version, migration.name
            )
        })?;
    }

    Ok(())
}

async fn get_current_version(db: &Surreal<Any>) -> Result<u32> {
    let result: Vec<SchemaVersion> = db
        .query("SELECT * FROM schema_version ORDER BY version DESC LIMIT 1")
        .await
        .context("Failed to query schema_version")?
        .take(0)
        .unwrap_or_default();

    Ok(result.into_iter().next().map(|v| v.version).unwrap_or(0))
}

async fn apply_migration(db: &Surreal<Any>, migration: &Migration) -> Result<()> {
    let digest = Sha256::digest(migration.checksum_source.as_bytes());
    let checksum: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    tracing::info!(
        "Applying migration v{}: {} (checksum: {})",
        migration.version,
        migration.name,
        &checksum[..8]
    );

    if !migration.sql.trim().is_empty() {
        let response = db
            .query(migration.sql)
            .await
            .with_context(|| format!("SQL error in migration v{}", migration.version))?;
        response.check().with_context(|| {
            format!("Migration v{} was rejected by SurrealDB", migration.version)
        })?;
    }

    if let Some(repair) = migration.repair {
        repair(db).await?;
    }

    let applied_at = Datetime::default();
    db.query(
        "INSERT INTO schema_version { version: $version, migration_name: $name, applied_at: $applied_at, checksum: $checksum }",
    )
    .bind(("version", migration.version))
    .bind(("name", migration.name))
    .bind(("applied_at", applied_at))
    .bind(("checksum", checksum))
    .await
    .context("Failed to record migration version")?;

    tracing::info!("✓ Migration v{} applied successfully", migration.version);
    Ok(())
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct SchemaVersion {
    pub version: u32,
}

#[cfg(test)]
mod tests {
    use super::{
        MIGRATION_V11_SQL, MIGRATION_V12_SQL, MIGRATION_V13_SQL, MIGRATION_V15_SQL,
        MIGRATION_V16_SQL, MIGRATION_V17_SQL, MIGRATION_V18_SQL, MIGRATION_V19_SQL,
        MIGRATION_V20_SQL, MIGRATION_V21_SQL, MIGRATIONS, RawMemoryEnumRecord,
        RawMindMapEnumRecord, RawTaskStreamEnumRecord, apply_migration, inspect_legacy_enum_data,
        normalize_enum_value,
    };
    use crate::{MapType, TaskStreamStatus};
    use surrealdb::Surreal;
    use surrealdb::engine::any::connect;
    use surrealdb::types::{Datetime, RecordId};
    use uuid::Uuid;

    #[test]
    fn migration_v15_is_registered() {
        let last = MIGRATIONS.last().expect("at least one migration");
        assert_eq!(last.version, 21);
        assert_eq!(last.name, "embedding_executor_journal");
        assert_eq!(last.sql, MIGRATION_V21_SQL);

        let v18 = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 18)
            .expect("v18 migration remains registered");
        assert_eq!(v18.name, "task_stream_scope_unique_index");
        assert_eq!(v18.sql, MIGRATION_V18_SQL);

        let v17 = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 17)
            .expect("v17 migration remains registered");
        assert_eq!(v17.name, "dynamic_embedding_index_metadata");
        assert_eq!(v17.sql, MIGRATION_V17_SQL);

        let v15 = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 15)
            .expect("v15 migration remains registered");
        assert_eq!(v15.name, "enum_fields_as_strings");
        assert_eq!(v15.sql, MIGRATION_V15_SQL);

        let v16 = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 16)
            .expect("v16 migration remains registered");
        assert_eq!(v16.name, "palace_drawers_table");
        assert_eq!(v16.sql, MIGRATION_V16_SQL);
    }

    #[test]
    fn migration_v17_removes_static_embedding_indexes() {
        assert!(MIGRATION_V17_SQL.contains("REMOVE INDEX IF EXISTS entity_embedding_hnsw"));
        assert!(MIGRATION_V17_SQL.contains("REMOVE INDEX IF EXISTS memory_embedding_hnsw"));
        assert!(MIGRATION_V17_SQL.contains("DEFINE TABLE IF NOT EXISTS schema_metadata"));
    }

    #[test]
    fn migration_v18_scopes_task_stream_name_uniqueness() {
        assert!(
            MIGRATION_V18_SQL.contains("REMOVE INDEX IF EXISTS task_stream_name ON task_stream")
        );
        assert!(MIGRATION_V18_SQL.contains("task_stream_scope_name"));
        assert!(MIGRATION_V18_SQL.contains("FIELDS agent_id, user_id, name UNIQUE"));
    }

    #[test]
    fn migration_v18_backfills_none_scopes_before_index() {
        // Pre-existing rows store agent_id/user_id as NONE; the new composite
        // unique index requires them normalized to "" so uniqueness is
        // consistently enforced. The backfill must run BEFORE the index DDL.
        assert!(
            MIGRATION_V18_SQL
                .contains("UPDATE task_stream SET agent_id = '' WHERE agent_id IS NONE")
        );
        assert!(
            MIGRATION_V18_SQL.contains("UPDATE task_stream SET user_id = '' WHERE user_id IS NONE")
        );
        let backfill_pos = MIGRATION_V18_SQL
            .find("UPDATE task_stream SET agent_id")
            .expect("backfill present");
        let define_pos = MIGRATION_V18_SQL
            .find("DEFINE INDEX")
            .expect("index DDL present");
        assert!(
            backfill_pos < define_pos,
            "backfill must precede index redefinition"
        );
    }

    #[test]
    fn migration_v19_defines_task_step_table_and_indexes() {
        assert!(MIGRATION_V19_SQL.contains("DEFINE TABLE IF NOT EXISTS task_step SCHEMAFULL"));
        for field in [
            "task_stream_id",
            "ordinal",
            "name",
            "description",
            "status",
            "idempotency_key",
            "result",
            "error",
            "started_at",
            "completed_at",
            "created_at",
        ] {
            assert!(
                MIGRATION_V19_SQL
                    .contains(&format!("DEFINE FIELD IF NOT EXISTS {field} ON task_step")),
                "v19 must define field '{field}' on task_step"
            );
        }
        assert!(MIGRATION_V19_SQL.contains("task_step_stream_ordinal"));
        assert!(MIGRATION_V19_SQL
            .contains("DEFINE INDEX IF NOT EXISTS task_step_idempotency_key ON task_step FIELDS idempotency_key UNIQUE"));
    }

    #[test]
    fn migration_v20_defines_idempotent_operation_ledger() {
        assert!(MIGRATION_V20_SQL.contains("DEFINE TABLE IF NOT EXISTS memory_operation"));
        assert!(MIGRATION_V20_SQL.contains(
            "DEFINE INDEX IF NOT EXISTS memory_operation_id ON memory_operation FIELDS operation_id UNIQUE"
        ));
        assert!(MIGRATION_V20_SQL.contains("memory_operation_event_sequence"));
        assert!(MIGRATION_V20_SQL.contains("memory_operation_part_identity"));
    }

    #[test]
    fn migration_v21_persists_executor_lifecycle() {
        for field in [
            "executor_progress_seq",
            "executor_exit_count",
            "executor_last_exit",
            "executor_error",
        ] {
            assert!(MIGRATION_V21_SQL.contains(&format!(
                "DEFINE FIELD IF NOT EXISTS {field} ON memory_operation"
            )));
        }
        assert!(MIGRATION_V21_SQL.contains("DEFINE TABLE IF NOT EXISTS memory_executor_event"));
        assert!(MIGRATION_V21_SQL.contains("memory_executor_event_identity"));
    }

    #[test]
    fn migration_v12_covers_nested_mindmap_fields() {
        assert!(MIGRATION_V11_SQL.contains("DEFINE FIELD nodes ON mindmap"));
        assert!(MIGRATION_V12_SQL.contains("DEFINE FIELD IF NOT EXISTS nodes.* ON mindmap"));
        assert!(MIGRATION_V12_SQL.contains("DEFINE FIELD IF NOT EXISTS edges.* ON mindmap"));
        assert!(MIGRATION_V12_SQL.contains("DEFINE FIELD IF NOT EXISTS nodes.*.metadata"));
        assert!(!MIGRATION_V12_SQL.contains("FLEXIBLE"));
    }

    #[test]
    fn migration_v13_covers_edge_fields() {
        assert!(MIGRATION_V13_SQL.contains("DEFINE FIELD IF NOT EXISTS edges.*.from_id"));
        assert!(MIGRATION_V13_SQL.contains("DEFINE FIELD IF NOT EXISTS edges.*.to_id"));
        assert!(MIGRATION_V13_SQL.contains("DEFINE FIELD IF NOT EXISTS edges.*.label"));
        assert!(MIGRATION_V13_SQL.contains("DEFINE FIELD IF NOT EXISTS edges.*.directed"));
    }

    #[test]
    fn normalize_enum_value_accepts_legacy_object_shape() {
        let record = RecordId::new("mindmap", "persona");
        let raw = serde_json::json!({"Radial": {}});
        let normalized = normalize_enum_value("mindmap", "map_type", &record, &raw, |value| {
            MapType::parse_str(value).map(|parsed| parsed.as_str().to_string())
        })
        .expect("legacy enum normalizes");

        assert_eq!(normalized.as_deref(), Some("radial"));
    }

    #[test]
    fn normalize_enum_value_rejects_unknown_value() {
        let record = RecordId::new("task_stream", "alpha");
        let raw = serde_json::json!({"UnknownStatus": {}});
        let issue = normalize_enum_value("task_stream", "status", &record, &raw, |value| {
            TaskStreamStatus::parse_str(value).map(|parsed| parsed.as_str().to_string())
        })
        .expect_err("unknown status should fail");

        assert_eq!(issue.table, "task_stream");
        assert_eq!(issue.field, "status");
    }

    #[tokio::test]
    async fn repair_migration_normalizes_legacy_rows() {
        let db = make_test_db().await;

        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 13)
        {
            apply_migration(&db, migration)
                .await
                .expect("apply setup migration");
        }

        seed_legacy_rows(&db).await;

        let report = inspect_legacy_enum_data(&db)
            .await
            .expect("inspect legacy rows");
        assert_eq!(report.planned_repairs(), 4);
        assert!(!report.has_issues());

        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version >= 14)
        {
            apply_migration(&db, migration)
                .await
                .expect("apply repair migration");
        }

        let repaired_memory: Vec<RawMemoryEnumRecord> = db
            .query("SELECT id, scope, memory_type FROM memory")
            .await
            .expect("query memory")
            .take(0)
            .unwrap_or_default();
        assert_eq!(repaired_memory.len(), 1);
        assert_eq!(
            repaired_memory[0].scope,
            serde_json::Value::String("global".to_string())
        );
        assert_eq!(
            repaired_memory[0].memory_type,
            serde_json::Value::String("semantic".to_string())
        );

        let repaired_streams: Vec<RawTaskStreamEnumRecord> = db
            .query("SELECT id, status FROM task_stream")
            .await
            .expect("query task_stream")
            .take(0)
            .unwrap_or_default();
        assert_eq!(
            repaired_streams[0].status,
            serde_json::Value::String("active".to_string())
        );

        let repaired_maps: Vec<RawMindMapEnumRecord> = db
            .query("SELECT id, map_type FROM mindmap")
            .await
            .expect("query mindmap")
            .take(0)
            .unwrap_or_default();
        assert_eq!(
            repaired_maps[0].map_type,
            serde_json::Value::String("radial".to_string())
        );
    }

    async fn make_test_db() -> Surreal<surrealdb::engine::any::Any> {
        let path = std::env::temp_dir().join(format!("migration-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        let db = connect(format!("rocksdb://{}", path.display()))
            .await
            .expect("connect embedded surrealdb");
        db.use_ns("test").use_db("test").await.expect("use ns/db");
        db
    }

    async fn seed_legacy_rows(db: &Surreal<surrealdb::engine::any::Any>) {
        let now = Datetime::default();

        let response = db
            .query(
                "CREATE memory:test_memory CONTENT {
                    content: $content,
                    embedding: NONE,
                    scope: { Global: {} },
                    memory_type: { Semantic: {} },
                    user_id: $user_id,
                    session_id: NONE,
                    agent_id: NONE,
                    task_stream_id: NONE,
                    categories: [],
                    metadata: NONE,
                    token_count: NONE,
                    importance: 0.5,
                    access_count: 0,
                    last_accessed_at: NONE,
                    valid_until: NONE,
                    version: 1,
                    created_at: $now,
                    updated_at: $now
                }",
            )
            .bind(("content", "legacy memory"))
            .bind(("user_id", "user-1"))
            .bind(("now", now))
            .await
            .expect("seed memory");
        response.check().expect("memory seed accepted");

        let response = db
            .query(
                "CREATE task_stream:test_stream CONTENT {
                    name: $name,
                    description: $description,
                    agent_id: $agent_id,
                    user_id: $user_id,
                    status: { Active: {} },
                    total_tokens: 0,
                    auto_summarize: true,
                    summary_count: 0,
                    model_id: NONE,
                    created_at: $now,
                    last_active: $now
                }",
            )
            .bind(("name", "legacy-stream"))
            .bind(("description", "legacy stream"))
            .bind(("agent_id", "agent-1"))
            .bind(("user_id", "user-1"))
            .bind(("now", now))
            .await
            .expect("seed task_stream");
        response.check().expect("task_stream seed accepted");

        let response = db
            .query(
                "CREATE mindmap:test_map CONTENT {
                    name: $name,
                    description: $description,
                    map_type: { Radial: {} },
                    agent_id: $agent_id,
                    user_id: $user_id,
                    task_stream_id: NONE,
                    tags: [],
                    nodes: [],
                    edges: [],
                    created_at: $now,
                    updated_at: $now
                }",
            )
            .bind(("name", "legacy-map"))
            .bind(("description", "legacy map"))
            .bind(("agent_id", "agent-1"))
            .bind(("user_id", "user-1"))
            .bind(("now", now))
            .await
            .expect("seed mindmap");
        response.check().expect("mindmap seed accepted");
    }
}
