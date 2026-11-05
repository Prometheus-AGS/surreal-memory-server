//! Memory expiry: the live view, the physical view, and an honest TTL sweep
//! (issue #52).
//!
//! A memory is *expired* when `valid_until < cutoff`; equality is still live.
//! The cutoff is read once per operation from the storage clock and bound as
//! `$now`, so one operation never sees two different "now"s and tests can use
//! a fixed clock. Normal reads use the live view (`LIVE_MEMORY_SQL`, applied in
//! the query before any top-k). Cleanup, erasure and audit use the physical
//! view, so hiding an expired row never prevents deleting it.
//!
//! Child module of `surreal` so it can use the storage internals directly.

use super::*;
use crate::compression::FaultPoint;
use crate::storage::SweepReport;
use chrono::{DateTime, Utc};

/// Most rows one sweep pass removes. A larger backlog drains over successive
/// passes instead of holding one unbounded query open.
const SWEEP_BATCH: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct CountRow {
    count: u64,
}

fn utc(value: Datetime) -> DateTime<Utc> {
    value.into()
}

impl SurrealStorage {
    /// The instant this operation treats as "now" for expiry.
    pub(crate) fn cutoff(&self) -> Datetime {
        Datetime::from(self.compression.clock.now())
    }

    /// True when `memory.valid_until` is strictly before `cutoff`.
    pub(crate) fn is_expired_at(memory: &Memory, cutoff: Datetime) -> bool {
        memory
            .valid_until
            .is_some_and(|valid_until| utc(valid_until) < utc(cutoff))
    }

    /// A memory by id regardless of expiry, for cleanup, deletion and audit.
    /// Normal reads use [`MemoryStorage::get_memory`], which hides expired rows.
    pub async fn get_memory_including_expired(&self, id: &str) -> Result<Option<Memory>> {
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

    /// Memories matching the owner filters. `live` applies the live view
    /// (active and unexpired); otherwise every physical row matches, which is
    /// what erasure needs.
    pub(crate) async fn list_memories_scoped(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        live: bool,
    ) -> Result<Vec<Memory>> {
        let mut parts: Vec<&str> = Vec::new();
        if user_id.is_some() {
            parts.push("user_id = $user_id");
        }
        if agent_id.is_some() {
            parts.push("agent_id = $agent_id");
        }
        if session_id.is_some() {
            parts.push("session_id = $session_id");
        }
        if live {
            parts.push(LIVE_MEMORY_SQL);
        }
        if parts.is_empty() {
            parts.push("true");
        }
        let sql = format!("SELECT * FROM memory WHERE {}", parts.join(" AND "));

        let db = self.live_db()?;
        let mut q = db.query(sql);
        if live {
            q = q.bind(("now", self.cutoff()));
        }
        if let Some(v) = user_id {
            q = q.bind(("user_id", v.to_string()));
        }
        if let Some(v) = agent_id {
            q = q.bind(("agent_id", v.to_string()));
        }
        if let Some(v) = session_id {
            q = q.bind(("session_id", v.to_string()));
        }
        let rows: Vec<DbMemory> = q.await?.take(0)?;
        Self::decode_memories(rows)
    }

    /// One TTL pass: physically select expired rows, delete each one in its own
    /// guarded transaction, and report what actually happened.
    ///
    /// A selection or decode failure is an error, never a zero-row success. A
    /// failed delete leaves the row in place (still hidden from normal reads)
    /// and is counted as `failed`, so the next pass retries it.
    pub(crate) async fn sweep_expired(&self) -> Result<SweepReport> {
        let db = self.live_db()?;
        let cutoff = self.cutoff();
        let rows: Vec<DbMemory> = db
            .query(format!(
                "SELECT * FROM memory WHERE valid_until IS NOT NONE AND valid_until < $now \
                 ORDER BY valid_until ASC LIMIT {SWEEP_BATCH}"
            ))
            .bind(("now", cutoff))
            .await?
            .take(0)?;
        let expired = Self::decode_memories(rows)?;

        let mut report = SweepReport {
            selected: expired.len() as u64,
            ..SweepReport::default()
        };
        for memory in &expired {
            let Some(id) = &memory.id else {
                report.failed += 1;
                continue;
            };
            if self.compression.fault(FaultPoint::SweepDelete) {
                tracing::warn!("injected fault: expired memory not deleted");
                report.failed += 1;
                continue;
            }
            match self.delete_if_still_expired(id, cutoff).await {
                Ok(true) => report.removed += 1,
                Ok(false) => report.skipped += 1,
                Err(error) => {
                    // The memory text is deliberately not logged.
                    tracing::warn!(%error, memory = %Self::record_id_to_string(id), "failed to delete an expired memory");
                    report.failed += 1;
                }
            }
        }
        Ok(report)
    }

    /// Delete one expired memory and write its audit row in a single
    /// transaction that re-checks expiry, so a row renewed after selection is
    /// left alone. `Ok(true)` only when this call removed the row.
    async fn delete_if_still_expired(&self, id: &RecordId, cutoff: Datetime) -> Result<bool> {
        let db = self.live_db()?;
        self.write_retrying_conflicts("expire_stale_memories", || async {
            check_statements(
                db.query(
                    "BEGIN TRANSACTION;\n\
                     LET $cur = (SELECT version, content, valid_until FROM ONLY $rid);\n\
                     IF $cur != NONE AND $cur.valid_until != NONE AND $cur.valid_until < $now {\n\
                       INSERT INTO memory_history { memory_id: $rid, version: $cur.version + 1, \
                         old_content: $cur.content, new_content: $cur.content, \
                         changed_at: $now, change_type: 'deleted' };\n\
                       DELETE ONLY $rid;\n\
                     };\n\
                     COMMIT TRANSACTION;",
                )
                .bind(("rid", id.clone()))
                .bind(("now", cutoff))
                .await?,
            )
        })
        .await?;

        // The transaction is a no-op when the row was renewed or already gone,
        // so success is confirmed by this call's own audit row.
        let marker: Vec<CountRow> = db
            .query(
                "SELECT count() FROM memory_history WHERE memory_id = $rid \
                 AND change_type = 'deleted' AND changed_at = $now GROUP ALL",
            )
            .bind(("rid", id.clone()))
            .bind(("now", cutoff))
            .await?
            .take(0)?;
        Ok(marker.first().is_some_and(|row| row.count > 0))
    }
}
