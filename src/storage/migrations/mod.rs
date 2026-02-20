//! Migration runner for surreal-memory schema evolution.
//!
//! Each migration is additive — never destructive. The runner reads the
//! highest applied version from the `schema_version` table and runs only
//! pending migrations in order. Safe to call at every startup.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::types::Datetime;

/// A single versioned schema migration.
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// All registered migrations in ascending version order.
/// Add new migrations to this list; never reorder or remove existing ones.
static MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial_entity_relation_schema",
    sql: MIGRATION_V1_SQL,
}];

const MIGRATION_V1_SQL: &str = "
-- ====================================================================
-- Migration v1: Baseline — entity + relation knowledge graph schema.
-- This codifies the schema that existed before the migration system.
-- ====================================================================

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

-- Version tracking table
DEFINE TABLE IF NOT EXISTS schema_version SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version ON schema_version TYPE int;
DEFINE FIELD IF NOT EXISTS migration_name ON schema_version TYPE string;
DEFINE FIELD IF NOT EXISTS applied_at ON schema_version TYPE datetime;
DEFINE FIELD IF NOT EXISTS checksum ON schema_version TYPE string;
";

/// Runs all pending migrations against the provided SurrealDB instance.
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
    // schema_version may not exist yet on first run — that's fine, return 0.
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

    // Execute the migration SQL
    db.query(migration.sql)
        .await
        .with_context(|| format!("SQL error in migration v{}", migration.version))?;

    // Record the applied version
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

use surrealdb_types::SurrealValue;

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct SchemaVersion {
    pub version: u32,
}
