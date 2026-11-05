//! Ownership-qualified memory mutations (issue #51).
//!
//! A caller restricted to one `user_id` must not be able to change or delete
//! another user's memory by guessing its ID. Checking ownership with a read and
//! then mutating an unqualified ID would race with a concurrent change, so the
//! ownership predicate is evaluated inside the same transaction as the write.
//! A memory that is missing and a memory owned by someone else are reported
//! identically, so the response does not reveal that the record exists.
//!
//! Child module of `surreal` so it can use the storage internals directly.

use super::*;

impl SurrealStorage {
    pub(crate) async fn update_memory_owned_impl(
        &self,
        id: &str,
        owner_user_id: &str,
        content: String,
    ) -> Result<Memory> {
        let db = self.live_db()?;
        let (table, key) = Self::parse_record_id_str(id, "memory")?;

        // Cheap pre-check so an unowned ID is rejected before paying for an
        // embedding. It is not authoritative: the transaction below is.
        let current = self.get_memory(id).await?;
        if current.is_none_or(|memory| memory.user_id.as_deref() != Some(owner_user_id)) {
            anyhow::bail!("Memory '{id}' not found");
        }

        let new_emb = self.embed_text(&content).await?;
        let token_count = Self::estimate_tokens(&content);
        let now = Datetime::default();

        self.write_retrying_conflicts("update_memory_owned", || async {
            check_statements(
                db.query(
                    "BEGIN TRANSACTION;\n\
                     LET $before = (SELECT content, version, user_id FROM ONLY type::record($table, $key));\n\
                     IF $before = NONE OR $before.user_id != $owner { THROW 'memory not found' };\n\
                     UPDATE type::record($table, $key) \
                     SET content = $content, embedding = $emb, token_count = $tc, version = $before.version + 1, updated_at = $now;\n\
                     INSERT INTO memory_history { memory_id: type::record($table, $key), version: $before.version + 1, old_content: $before.content, new_content: $content, changed_at: $now, change_type: 'updated' };\n\
                     COMMIT TRANSACTION;",
                )
                .bind(("table", table.clone()))
                .bind(("key", key.clone()))
                .bind(("owner", owner_user_id.to_string()))
                .bind(("content", content.clone()))
                .bind(("emb", new_emb.clone()))
                .bind(("tc", token_count))
                .bind(("now", now))
                .await?,
            )
        })
        .await
        .map_err(|error| not_found_if_rejected(error, id))?;

        self.get_memory(id)
            .await?
            .context("Failed to update memory")
    }

    pub(crate) async fn delete_memory_owned_impl(
        &self,
        id: &str,
        owner_user_id: &str,
    ) -> Result<()> {
        let db = self.live_db()?;
        let (table, key) = Self::parse_record_id_str(id, "memory")?;
        let now = Datetime::default();

        // Fast, non-authoritative rejection so the common "not yours" case is a
        // plain not-found. The transaction below still decides.
        let current = self.get_memory(id).await?;
        if current.is_none_or(|memory| memory.user_id.as_deref() != Some(owner_user_id)) {
            anyhow::bail!("Memory '{id}' not found");
        }

        self.write_retrying_conflicts("delete_memory_owned", || async {
            check_statements(
                db.query(
                    "BEGIN TRANSACTION;\n\
                     LET $row = (SELECT id, content, version, user_id FROM ONLY type::record($table, $key));\n\
                     IF $row = NONE OR $row.user_id != $owner { THROW 'memory not found' };\n\
                     INSERT INTO memory_history { memory_id: $row.id, version: $row.version + 1, old_content: $row.content, new_content: $row.content, changed_at: $now, change_type: 'deleted' };\n\
                     DELETE type::record($table, $key);\n\
                     COMMIT TRANSACTION;",
                )
                .bind(("table", table.clone()))
                .bind(("key", key.clone()))
                .bind(("owner", owner_user_id.to_string()))
                .bind(("now", now))
                .await?,
            )
        })
        .await
        .map_err(|error| not_found_if_rejected(error, id))?;
        Ok(())
    }
}

/// The transaction rejects a row that vanished or changed owner after the
/// pre-check with `THROW 'memory not found'`; report it as an ordinary
/// not-found so the caller cannot tell the cases apart.
fn not_found_if_rejected(error: anyhow::Error, id: &str) -> anyhow::Error {
    if format!("{error:#}")
        .to_lowercase()
        .contains("memory not found")
    {
        anyhow::anyhow!("Memory '{id}' not found")
    } else {
        error
    }
}
