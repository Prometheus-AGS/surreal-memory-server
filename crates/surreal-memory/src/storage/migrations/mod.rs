//! Migration runner for surreal-memory schema evolution.
//!
//! Migrations are additive — never destructive. The runner reads the highest
//! applied version from `schema_version` and runs only pending migrations.
//! Safe to call at every startup.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::types::Datetime;
use surrealdb_types::SurrealValue;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

static MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_entity_relation_schema",
        sql: MIGRATION_V1_SQL,
    },
    Migration {
        version: 2,
        name: "scoped_memory_table",
        sql: MIGRATION_V2_SQL,
    },
    Migration {
        version: 3,
        name: "task_stream_table",
        sql: MIGRATION_V3_SQL,
    },
    Migration {
        version: 4,
        name: "memory_history_table",
        sql: MIGRATION_V4_SQL,
    },
    Migration {
        version: 5,
        name: "hnsw_vector_indexes",
        sql: MIGRATION_V5_SQL,
    },
    Migration {
        version: 6,
        name: "mindmap_table_and_fulltext_indexes",
        sql: MIGRATION_V6_SQL,
    },
    Migration {
        version: 7,
        name: "task_stream_auto_summarization_fields",
        sql: MIGRATION_V7_SQL,
    },
    Migration {
        version: 8,
        name: "memory_metadata_flexible",
        sql: MIGRATION_V8_SQL,
    },
    Migration {
        version: 9,
        name: "mindmap_nodes_edges_flexible",
        sql: MIGRATION_V9_SQL,
    },
    Migration {
        version: 10,
        name: "mindmap_nodes_edges_flexible_overwrite",
        sql: MIGRATION_V10_SQL,
    },
    Migration {
        version: 11,
        name: "mindmap_nodes_edges_remove_redefine",
        sql: MIGRATION_V11_SQL,
    },
];

// ── v1: Baseline ─────────────────────────────────────────────────────────────

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

// ── v2: Scoped Memory (mem0-compatible) ──────────────────────────────────────

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

// ── v3: TaskStream ────────────────────────────────────────────────────────────

const MIGRATION_V3_SQL: &str = "
DEFINE TABLE IF NOT EXISTS task_stream SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON task_stream TYPE string;
DEFINE FIELD IF NOT EXISTS description ON task_stream TYPE option<string>;
DEFINE FIELD IF NOT EXISTS agent_id ON task_stream TYPE option<string>;
DEFINE FIELD IF NOT EXISTS user_id ON task_stream TYPE option<string>;
DEFINE FIELD IF NOT EXISTS status ON task_stream TYPE any DEFAULT 'active';
DEFINE FIELD IF NOT EXISTS total_tokens ON task_stream TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON task_stream TYPE datetime;
DEFINE FIELD IF NOT EXISTS last_active ON task_stream TYPE datetime;
DEFINE INDEX IF NOT EXISTS task_stream_name ON task_stream FIELDS name UNIQUE;
";

// ── v4: MemoryHistory (audit log) ─────────────────────────────────────────────

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

// ── v5: HNSW vector indexes for ANN search ────────────────────────────────────

const MIGRATION_V5_SQL: &str = "
DEFINE INDEX IF NOT EXISTS entity_embedding_hnsw ON entity FIELDS embedding HNSW DIMENSION 1536 DIST COSINE TYPE F32;
DEFINE INDEX IF NOT EXISTS memory_embedding_hnsw ON memory FIELDS embedding HNSW DIMENSION 1536 DIST COSINE TYPE F32;
";

// ── v6: Mindmap table + BM25 full-text search indexes ────────────────────────

const MIGRATION_V6_SQL: &str = "
DEFINE TABLE IF NOT EXISTS mindmap SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON mindmap TYPE string;
DEFINE FIELD IF NOT EXISTS description ON mindmap TYPE option<string>;
DEFINE FIELD IF NOT EXISTS map_type ON mindmap TYPE any DEFAULT 'radial';
DEFINE FIELD IF NOT EXISTS agent_id ON mindmap TYPE option<string>;
DEFINE FIELD IF NOT EXISTS user_id ON mindmap TYPE option<string>;
DEFINE FIELD IF NOT EXISTS task_stream_id ON mindmap TYPE option<record<task_stream>>;
DEFINE FIELD IF NOT EXISTS tags ON mindmap TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS tags.* ON mindmap TYPE string;
DEFINE FIELD IF NOT EXISTS nodes ON mindmap TYPE array<object> DEFAULT [];
DEFINE FIELD IF NOT EXISTS edges ON mindmap TYPE array<object> DEFAULT [];
DEFINE FIELD IF NOT EXISTS created_at ON mindmap TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON mindmap TYPE datetime;
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
// Redefine as FLEXIBLE so nested object fields are unconstrained.
// Also drop and recreate the unique index to use NULLS NOT DISTINCT semantics
// so two mindmaps with the same name but different (or null) user_ids can coexist.

// Use OVERWRITE (not IF NOT EXISTS) so we force-update existing strict field
// definitions that were set in the v6 schema before FLEXIBLE was needed.
const MIGRATION_V9_SQL: &str = "
DEFINE FIELD OVERWRITE nodes ON mindmap TYPE array<object> FLEXIBLE DEFAULT [];
DEFINE FIELD OVERWRITE edges ON mindmap TYPE array<object> FLEXIBLE DEFAULT [];
";

// ── v10: Force overwrite mindmap nodes/edges to FLEXIBLE ─────────────────────
//
// v9 used IF NOT EXISTS which silently skips already-defined fields.
// v10 uses OVERWRITE to guarantee the existing strict array<object> definition
// is replaced with FLEXIBLE, unblocking MindMapNode.metadata serialization.

const MIGRATION_V10_SQL: &str = "
DEFINE FIELD OVERWRITE nodes ON mindmap TYPE array<object> FLEXIBLE DEFAULT [];
DEFINE FIELD OVERWRITE edges ON mindmap TYPE array<object> FLEXIBLE DEFAULT [];
";

// ── v11: Remove and redefine mindmap nodes/edges as FLEXIBLE ─────────────────
//
// OVERWRITE alone does not clear existing sub-field constraints in SurrealDB 3.x.
// The v6 schema defined nodes/edges as strict array<object> which rejects any
// object field not explicitly known to the schema (e.g. nodes[0].color).
// Fix: REMOVE the fields entirely to purge sub-field metadata, then redefine
// them clean as FLEXIBLE so MindMapNode can carry arbitrary fields.

const MIGRATION_V11_SQL: &str = "
REMOVE FIELD IF EXISTS nodes ON mindmap;
REMOVE FIELD IF EXISTS edges ON mindmap;
DEFINE FIELD nodes ON mindmap TYPE array<object> FLEXIBLE DEFAULT [];
DEFINE FIELD edges ON mindmap TYPE array<object> FLEXIBLE DEFAULT [];
";

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
    let checksum = format!("{:x}", Sha256::digest(migration.sql.as_bytes()));
    tracing::info!(
        "Applying migration v{}: {} (checksum: {})",
        migration.version,
        migration.name,
        &checksum[..8]
    );

    db.query(migration.sql)
        .await
        .with_context(|| format!("SQL error in migration v{}", migration.version))?;

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
