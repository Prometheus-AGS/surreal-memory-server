//! `PalaceAdapter` — implements mempalace-core's `StorageBackend` over a shared
//! `Surreal<Any>` handle, reusing the same SurrealDB connection that powers the
//! rest of surreal-memory.
//!
//! The adapter delegates to SurrealQL queries against the `drawers` table
//! (created by migration v16).

use anyhow::{Context, Result};
use async_trait::async_trait;
use mempalace_core::storage::{
    types::{Drawer, DrawerFilter, DrawerHit, RoomStats, WingStats},
    StorageBackend,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::types::RecordId;
use uuid::Uuid;

/// Closure type that yields a live DB handle (resilient to reconnection).
type DbFn = Arc<dyn Fn() -> Result<Surreal<Any>> + Send + Sync>;

/// Implements `StorageBackend` by forwarding to SurrealDB via a shared `Surreal<Any>`.
pub struct PalaceAdapter {
    db_fn: DbFn,
}

impl PalaceAdapter {
    /// Create a new adapter.
    ///
    /// `db_fn` is called on every operation so the adapter automatically uses
    /// a reconnected handle when the underlying connection recovers.
    pub fn new(db_fn: impl Fn() -> Result<Surreal<Any>> + Send + Sync + 'static) -> Self {
        Self {
            db_fn: Arc::new(db_fn),
        }
    }

    fn db(&self) -> Result<Surreal<Any>> {
        (self.db_fn)()
    }
}

// ── SurrealDB helper: raw drawer for deserialization ─────────────────────────
//
// SurrealDB returns record IDs as objects (`{ tb: "drawers", id: { String: "..." } }`)
// so we deserialize via a helper and convert to the flat `Drawer` type.

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawDrawer {
    id: Option<RecordId>,
    content: String,
    wing: String,
    room: String,
    hall: String,
    source_file: Option<String>,
    date: Option<String>,
    importance: f32,
    embedding: Option<Vec<f32>>,
    content_hash: Option<String>,
}

impl RawDrawer {
    fn into_drawer(self) -> Drawer {
        let id = self
            .id
            .map(|r| r.key().to_string())
            .unwrap_or_default();
        Drawer {
            id,
            content: self.content,
            wing: self.wing,
            room: self.room,
            hall: self.hall,
            source_file: self.source_file,
            date: self.date,
            importance: self.importance,
            embedding: self.embedding,
        }
    }
}

/// Compute SHA-256 content hash for deduplication.
fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

#[async_trait]
impl StorageBackend for PalaceAdapter {
    // ── Write ────────────────────────────────────────────────────────────────

    async fn add_drawer(&self, drawer: Drawer) -> Result<String> {
        let db = self.db()?;
        let id = if drawer.id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            drawer.id.clone()
        };
        let content_hash = sha256_hex(&drawer.content);

        db.query(
            "CREATE type::thing('drawers', $id) CONTENT {
                content: $content,
                wing: $wing,
                room: $room,
                hall: $hall,
                source_file: $source_file,
                date: $date,
                importance: $importance,
                embedding: $embedding,
                content_hash: $content_hash
            }",
        )
        .bind(("id", &id))
        .bind(("content", &drawer.content))
        .bind(("wing", &drawer.wing))
        .bind(("room", &drawer.room))
        .bind(("hall", &drawer.hall))
        .bind(("source_file", &drawer.source_file))
        .bind(("date", &drawer.date))
        .bind(("importance", drawer.importance))
        .bind(("embedding", &drawer.embedding))
        .bind(("content_hash", &content_hash))
        .await
        .context("PalaceAdapter::add_drawer CREATE failed")?;

        Ok(id)
    }

    async fn delete_drawer(&self, id: &str) -> Result<()> {
        let db = self.db()?;
        db.query("DELETE type::thing('drawers', $id)")
            .bind(("id", id))
            .await
            .context("PalaceAdapter::delete_drawer failed")?;
        Ok(())
    }

    // ── Read ─────────────────────────────────────────────────────────────────

    async fn get_drawer(&self, id: &str) -> Result<Option<Drawer>> {
        let db = self.db()?;
        let mut resp = db
            .query("SELECT * FROM type::thing('drawers', $id)")
            .bind(("id", id))
            .await
            .context("PalaceAdapter::get_drawer failed")?;
        let rows: Vec<RawDrawer> = resp.take(0)?;
        Ok(rows.into_iter().next().map(RawDrawer::into_drawer))
    }

    async fn list_drawers(
        &self,
        filter: DrawerFilter,
        limit: usize,
    ) -> Result<Vec<Drawer>> {
        let db = self.db()?;

        // Build dynamic WHERE clause
        let mut conditions: Vec<String> = Vec::new();
        if let Some(ref w) = filter.wing {
            conditions.push(format!("wing = '{}'", w.replace('\'', "''")));
        }
        if let Some(ref r) = filter.room {
            conditions.push(format!("room = '{}'", r.replace('\'', "''")));
        }
        if let Some(ref h) = filter.hall {
            conditions.push(format!("hall = '{}'", h.replace('\'', "''")));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT * FROM drawers{} ORDER BY importance DESC LIMIT {}",
            where_clause, limit
        );

        let mut resp = db
            .query(&sql)
            .await
            .context("PalaceAdapter::list_drawers failed")?;
        let rows: Vec<RawDrawer> = resp.take(0)?;
        Ok(rows.into_iter().map(RawDrawer::into_drawer).collect())
    }

    async fn search_drawers(
        &self,
        query_embedding: &[f32],
        filter: DrawerFilter,
        n: usize,
    ) -> Result<Vec<DrawerHit>> {
        let db = self.db()?;

        // Build optional filter conditions
        let mut conditions: Vec<String> = Vec::new();
        if let Some(ref w) = filter.wing {
            conditions.push(format!("wing = '{}'", w.replace('\'', "''")));
        }
        if let Some(ref r) = filter.room {
            conditions.push(format!("room = '{}'", r.replace('\'', "''")));
        }
        if let Some(ref h) = filter.hall {
            conditions.push(format!("hall = '{}'", h.replace('\'', "''")));
        }

        let extra_where = if conditions.is_empty() {
            String::new()
        } else {
            format!(" AND {}", conditions.join(" AND "))
        };

        // Use SurrealDB KNN operator with the MTREE index
        let sql = format!(
            "SELECT *, vector::similarity::cosine(embedding, $vec) AS similarity \
             FROM drawers \
             WHERE embedding <|{n}|> $vec{extra_where} \
             ORDER BY similarity DESC"
        );

        let mut resp = db
            .query(&sql)
            .bind(("vec", query_embedding.to_vec()))
            .await
            .context("PalaceAdapter::search_drawers KNN query failed")?;

        #[derive(Deserialize)]
        struct HitRow {
            #[serde(flatten)]
            drawer: RawDrawer,
            similarity: f32,
        }

        let rows: Vec<HitRow> = resp.take(0)?;
        Ok(rows
            .into_iter()
            .map(|r| DrawerHit {
                drawer: r.drawer.into_drawer(),
                similarity: r.similarity,
            })
            .collect())
    }

    async fn check_duplicate(&self, content_hash: &str) -> Result<bool> {
        let db = self.db()?;
        let mut resp = db
            .query("SELECT count() AS cnt FROM drawers WHERE content_hash = $hash GROUP ALL")
            .bind(("hash", content_hash))
            .await
            .context("PalaceAdapter::check_duplicate failed")?;

        #[derive(Deserialize)]
        struct CountRow {
            cnt: u64,
        }

        let rows: Vec<CountRow> = resp.take(0)?;
        Ok(rows.first().map(|r| r.cnt > 0).unwrap_or(false))
    }

    // ── Taxonomy ─────────────────────────────────────────────────────────────

    async fn list_wings(&self) -> Result<Vec<WingStats>> {
        let db = self.db()?;
        let mut resp = db
            .query(
                "SELECT wing AS name, count() AS count \
                 FROM drawers GROUP BY wing ORDER BY count DESC",
            )
            .await
            .context("PalaceAdapter::list_wings failed")?;
        let rows: Vec<WingStats> = resp.take(0)?;
        Ok(rows)
    }

    async fn list_rooms(&self, wing: Option<&str>) -> Result<Vec<RoomStats>> {
        let db = self.db()?;
        let sql = if let Some(w) = wing {
            format!(
                "SELECT room AS name, wing, count() AS count \
                 FROM drawers WHERE wing = '{}' GROUP BY room, wing ORDER BY count DESC",
                w.replace('\'', "''")
            )
        } else {
            "SELECT room AS name, wing, count() AS count \
             FROM drawers GROUP BY room, wing ORDER BY count DESC"
                .to_string()
        };
        let mut resp = db
            .query(&sql)
            .await
            .context("PalaceAdapter::list_rooms failed")?;
        let rows: Vec<RoomStats> = resp.take(0)?;
        Ok(rows)
    }

    async fn drawer_count(&self) -> Result<usize> {
        let db = self.db()?;
        let mut resp = db
            .query("SELECT count() AS cnt FROM drawers GROUP ALL")
            .await
            .context("PalaceAdapter::drawer_count failed")?;

        #[derive(Deserialize)]
        struct CountRow {
            cnt: u64,
        }

        let rows: Vec<CountRow> = resp.take(0)?;
        Ok(rows.first().map(|r| r.cnt as usize).unwrap_or(0))
    }
}
