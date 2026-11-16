//! Bounded, resumable memory enumeration (issue #62).
//!
//! Every memory listing is a keyset page: `id > $after ORDER BY id ASC LIMIT n`.
//! Record keys are unique and immutable, so a memory present for a whole
//! iteration is returned exactly once without any server-side cursor state.
//! The owner filters and the live view are applied on every page, so a cursor
//! only ever chooses a starting position inside the caller's own scope.
//!
//! Each page takes two queries: a pre-pass reads only IDs and stored sizes,
//! the page accepts rows up to the byte budget, and the second query fetches
//! just that ID range. So no page asks the database for more than the budget
//! plus one memory, whatever the row limit.
//!
//! Failures are described by the request (operation, page size, embedding
//! flag, which owner filters were set), never by filter values or content.
//!
//! Child module of `surreal` so it can use the storage internals directly.

use super::*;
use crate::compression::FaultPoint;
use crate::storage::page::{
    DEFAULT_MEMORY_PAGE_SIZE, MAX_MEMORY_PAGE_SIZE, MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS,
    MEMORY_PAGE_BYTE_BUDGET, MemoryPage, MemoryPageRequest,
};
use std::fmt;

/// IDs selected per batch by bulk deletion.
const DELETE_BATCH: usize = 200;

/// Prefix of every cursor: a cursor is the record ID of the last memory on the
/// previous page.
const CURSOR_PREFIX: &str = "memory:";

/// The owner filters of one listing.
#[derive(Clone, Copy)]
struct OwnerFilters<'a> {
    user_id: Option<&'a str>,
    agent_id: Option<&'a str>,
    session_id: Option<&'a str>,
}

impl<'a> OwnerFilters<'a> {
    fn of(request: &'a MemoryPageRequest) -> Self {
        Self {
            user_id: request.user_id.as_deref(),
            agent_id: request.agent_id.as_deref(),
            session_id: request.session_id.as_deref(),
        }
    }

    /// `WHERE` clauses and their bindings, in the same order.
    fn clauses(&self) -> (Vec<&'static str>, Vec<(&'static str, String)>) {
        let mut clauses = Vec::new();
        let mut binds = Vec::new();
        for (value, clause, name) in [
            (self.user_id, "user_id = $user_id", "user_id"),
            (self.agent_id, "agent_id = $agent_id", "agent_id"),
            (self.session_id, "session_id = $session_id", "session_id"),
        ] {
            if let Some(value) = value {
                clauses.push(clause);
                binds.push((name, value.to_string()));
            }
        }
        (clauses, binds)
    }
}

/// What a failed listing logs and reports: the request's shape only.
struct ListingContext {
    operation: &'static str,
    page_size: usize,
    include_embeddings: bool,
    user_id: bool,
    agent_id: bool,
    session_id: bool,
}

impl ListingContext {
    fn new(
        operation: &'static str,
        page_size: usize,
        include_embeddings: bool,
        filters: OwnerFilters<'_>,
    ) -> Self {
        Self {
            operation,
            page_size,
            include_embeddings,
            user_id: filters.user_id.is_some(),
            agent_id: filters.agent_id.is_some(),
            session_id: filters.session_id.is_some(),
        }
    }

    /// Log the failure at `warn` and wrap it with this context.
    fn fail(&self, error: anyhow::Error) -> anyhow::Error {
        tracing::warn!(
            operation = self.operation,
            page_size = self.page_size,
            include_embeddings = self.include_embeddings,
            user_id_filter = self.user_id,
            agent_id_filter = self.agent_id,
            session_id_filter = self.session_id,
            %error,
            "memory listing failed"
        );
        error.context(self.to_string())
    }

    /// Like [`fail`](Self::fail) for bulk deletion, stating the progress made.
    fn fail_after(&self, error: anyhow::Error, deleted: u64) -> anyhow::Error {
        tracing::warn!(
            operation = self.operation,
            batch_size = self.page_size,
            user_id_filter = self.user_id,
            agent_id_filter = self.agent_id,
            session_id_filter = self.session_id,
            deleted,
            %error,
            "bulk memory deletion failed"
        );
        error.context(format!(
            "{self} after deleting {deleted} memories; rerun once the cause is resolved"
        ))
    }
}

impl fmt::Display for ListingContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = |present: bool| if present { "set" } else { "unset" };
        write!(
            f,
            "{} failed (page_size={}, include_embeddings={}, filters: user_id={}, agent_id={}, session_id={})",
            self.operation,
            self.page_size,
            self.include_embeddings,
            set(self.user_id),
            set(self.agent_id),
            set(self.session_id),
        )
    }
}

/// The validated page size of `request`. Out-of-range sizes are rejected,
/// never clamped, so a caller always knows what it asked for.
fn page_size(request: &MemoryPageRequest) -> Result<usize> {
    let (max, qualifier) = if request.include_embeddings {
        (MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS, "with embeddings")
    } else {
        (MAX_MEMORY_PAGE_SIZE, "without embeddings")
    };
    let limit = request.limit.unwrap_or(DEFAULT_MEMORY_PAGE_SIZE.min(max));
    if limit == 0 || limit > max {
        anyhow::bail!(
            "invalid limit {limit}: the page size {qualifier} must be between 1 and {max}"
        );
    }
    Ok(limit)
}

/// The record a cursor points at. A value that is not a memory record ID is
/// an invalid request, never a silent restart from the first page.
fn parse_cursor(cursor: &str) -> Result<RecordId> {
    let key = cursor
        .strip_prefix(CURSOR_PREFIX)
        .filter(|key| !key.is_empty())
        .filter(|key| !key.chars().any(|c| c.is_whitespace() || c.is_control()));
    match key {
        Some(key) => Ok(RecordId::new("memory", key)),
        None => anyhow::bail!("invalid cursor: expected the next_cursor of a previous memory page"),
    }
}

/// Estimated encoded bytes of one memory's fields other than its content and
/// embedding (IDs, scope, timestamps, categories, small metadata, JSON keys).
const ROW_OVERHEAD_ESTIMATE: usize = 1024;
/// Estimated encoded bytes of one embedding value, separator included.
const EMBEDDING_VALUE_ESTIMATE: usize = 16;

/// One row of the size pre-pass: the record and the stored sizes that
/// dominate its encoded size. Content and vectors are not sent.
#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct RowSize {
    id: RecordId,
    content_bytes: u64,
    embedding_dims: u64,
}

impl RowSize {
    fn estimate(&self, include_embeddings: bool) -> usize {
        let content = usize::try_from(self.content_bytes).unwrap_or(usize::MAX);
        let embedding = if include_embeddings {
            usize::try_from(self.embedding_dims)
                .unwrap_or(usize::MAX)
                .saturating_mul(EMBEDDING_VALUE_ESTIMATE)
        } else {
            0
        };
        content
            .saturating_add(embedding)
            .saturating_add(ROW_OVERHEAD_ESTIMATE)
    }
}

/// The rows a page may fetch: up to `size`, stopping before the estimated
/// encoded size would exceed the budget, always at least one. Also says
/// whether any matching row remains after them.
fn accept_rows(sizes: &[RowSize], size: usize, include_embeddings: bool) -> (usize, bool) {
    // Brackets plus one separator per memory, as in `assemble_page`.
    let mut bytes = 1usize;
    let mut accepted = 0usize;
    for row in sizes.iter().take(size) {
        let estimate = row.estimate(include_embeddings).saturating_add(1);
        if accepted > 0 && bytes.saturating_add(estimate) > MEMORY_PAGE_BYTE_BUDGET {
            break;
        }
        bytes = bytes.saturating_add(estimate);
        accepted += 1;
    }
    (accepted, sizes.len() > accepted)
}

/// Build the page from the fetched rows, trimming again by their actual
/// encoded size (the estimate ignores most metadata, and a row can grow
/// between the two queries); always keeps one memory. `range_end` is the last
/// accepted record, where the next page resumes when nothing was trimmed.
fn assemble_page(
    rows: Vec<Memory>,
    more_after: bool,
    range_end: Option<&RecordId>,
) -> Result<MemoryPage> {
    let mut memories = Vec::with_capacity(rows.len());
    // Encoded size of the JSON array: brackets plus one separator per memory.
    let mut bytes = 1usize;
    let mut truncated = false;
    for memory in rows {
        let encoded = serde_json::to_vec(&memory)
            .context("memory page: a memory could not be encoded")?
            .len()
            + 1;
        if !memories.is_empty() && bytes + encoded > MEMORY_PAGE_BYTE_BUDGET {
            truncated = true;
            break;
        }
        bytes += encoded;
        memories.push(memory);
    }
    let has_more = more_after || truncated;
    let next_cursor = if truncated {
        let last = memories
            .last()
            .and_then(|memory| memory.id.as_ref())
            .context("memory page: a listed memory has no record id")?;
        Some(SurrealStorage::record_id_to_string(last))
    } else if more_after {
        let last = range_end.context("memory page: no accepted record to resume after")?;
        Some(SurrealStorage::record_id_to_string(last))
    } else {
        None
    };
    Ok(MemoryPage {
        memories,
        next_cursor,
        has_more,
    })
}

impl SurrealStorage {
    /// One page of live memories, with the live view's `$now` fixed by the
    /// caller so a multi-page operation sees one cutoff.
    ///
    /// Read in two queries so the database response stays within the byte
    /// budget plus one memory: first the IDs and stored sizes of up to
    /// `limit + 1` rows, then exactly the accepted ID range.
    pub(crate) async fn memory_page(
        &self,
        operation: &'static str,
        request: &MemoryPageRequest,
        cutoff: Datetime,
    ) -> Result<MemoryPage> {
        let size = page_size(request)?;
        let after = request.cursor.as_deref().map(parse_cursor).transpose()?;
        let filters = OwnerFilters::of(request);
        let include = request.include_embeddings;
        let context = ListingContext::new(operation, size, include, filters);
        let sizes = self
            .fetch_page_sizes(filters, after.as_ref(), size, cutoff)
            .await
            .map_err(|error| context.fail(error))?;
        let (accepted, more_after) = accept_rows(&sizes, size, include);
        let range_end = accepted.checked_sub(1).map(|last| &sizes[last].id);
        let rows = match range_end {
            Some(last) => self
                .fetch_page_rows(filters, after.as_ref(), last, accepted, include, cutoff)
                .await
                .map_err(|error| context.fail(error))?,
            None => Vec::new(),
        };
        let page = assemble_page(rows, more_after, range_end)?;
        if page.next_cursor.is_some() && page.next_cursor == request.cursor {
            // A record key that does not round-trip through the cursor would
            // otherwise repeat this page forever in a follow-the-cursor loop.
            return Err(context.fail(anyhow::anyhow!(
                "memory page did not advance past its cursor"
            )));
        }
        Ok(page)
    }

    /// The keyset `WHERE` clauses and bindings shared by both page queries.
    fn page_clauses(
        filters: OwnerFilters<'_>,
        after: Option<&RecordId>,
    ) -> (Vec<&'static str>, Vec<(&'static str, String)>) {
        let (mut clauses, binds) = filters.clauses();
        clauses.push(LIVE_MEMORY_SQL);
        if after.is_some() {
            clauses.push("id > $after");
        }
        (clauses, binds)
    }

    /// Pre-pass: IDs and stored sizes of up to `size + 1` live rows after the
    /// cursor, in key order. No content or vectors leave the database.
    async fn fetch_page_sizes(
        &self,
        filters: OwnerFilters<'_>,
        after: Option<&RecordId>,
        size: usize,
        cutoff: Datetime,
    ) -> Result<Vec<RowSize>> {
        let (clauses, binds) = Self::page_clauses(filters, after);
        let sql = format!(
            "SELECT id, bytes::len(<bytes>content) AS content_bytes, \
             array::len(embedding ?? []) AS embedding_dims \
             FROM memory WHERE {} ORDER BY id ASC LIMIT {}",
            clauses.join(" AND "),
            size + 1
        );
        let db = self.live_db()?;
        let mut query = db.query(sql).bind(("now", cutoff));
        for bind in binds {
            query = query.bind(bind);
        }
        if let Some(after) = after {
            query = query.bind(("after", after.clone()));
        }
        Ok(query.await?.take(0)?)
    }

    /// The accepted range `(after, last]`, at most `count` rows.
    async fn fetch_page_rows(
        &self,
        filters: OwnerFilters<'_>,
        after: Option<&RecordId>,
        last: &RecordId,
        count: usize,
        include_embeddings: bool,
        cutoff: Datetime,
    ) -> Result<Vec<Memory>> {
        let (mut clauses, binds) = Self::page_clauses(filters, after);
        clauses.push("id <= $last");
        // Vectors never leave the database unless they were asked for.
        let projection = if include_embeddings {
            "*"
        } else {
            "* OMIT embedding"
        };
        let sql = format!(
            "SELECT {projection} FROM memory WHERE {} ORDER BY id ASC LIMIT {count}",
            clauses.join(" AND ")
        );

        let db = self.live_db()?;
        let mut query = db
            .query(sql)
            .bind(("now", cutoff))
            .bind(("last", last.clone()));
        for bind in binds {
            query = query.bind(bind);
        }
        if let Some(after) = after {
            query = query.bind(("after", after.clone()));
        }
        let rows: Vec<DbMemory> = query.await?.take(0)?;
        Self::decode_memories(rows)
    }

    /// The legacy complete listing, fetched through bounded pages that share
    /// one cutoff. Any failed page fails the whole call.
    pub(crate) async fn all_memories_paged(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<Memory>> {
        let cutoff = self.cutoff();
        let mut request = MemoryPageRequest {
            user_id: user_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            session_id: session_id.map(str::to_string),
            limit: Some(MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS),
            cursor: None,
            include_embeddings: true,
        };
        let mut all = Vec::new();
        loop {
            let page = self
                .memory_page("get_all_memories", &request, cutoff)
                .await?;
            all.extend(page.memories);
            match page.next_cursor {
                Some(cursor) => request.cursor = Some(cursor),
                None => return Ok(all),
            }
        }
    }

    /// Delete every memory matching the owner filters, expired and archived
    /// ones included. Selects record IDs only, in ID-ordered batches, and
    /// deletes each through [`MemoryStorage::delete_memory`] so history is
    /// written as before. It stops at the first memory it cannot delete and
    /// reports how many were deleted first; once the cause is resolved, a rerun
    /// deletes the rest.
    pub(crate) async fn delete_all_paged(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<u64> {
        let filters = OwnerFilters {
            user_id,
            agent_id,
            session_id,
        };
        let context = ListingContext::new("delete_all_memories", DELETE_BATCH, false, filters);
        let mut after: Option<RecordId> = None;
        let mut deleted = 0u64;
        loop {
            let ids = self
                .memory_id_batch(filters, after.as_ref())
                .await
                .map_err(|error| context.fail_after(error, deleted))?;
            for id in &ids {
                let outcome = if self.compression.fault(FaultPoint::BulkDelete) {
                    Err(anyhow::anyhow!("injected fault: memory not deleted"))
                } else {
                    self.delete_memory(&Self::record_id_to_string(id)).await
                };
                if let Err(error) = outcome {
                    return Err(context.fail_after(error, deleted));
                }
                deleted += 1;
            }
            if ids.len() < DELETE_BATCH {
                return Ok(deleted);
            }
            after = ids.last().cloned();
        }
    }

    /// The next batch of record IDs (physical view: expired and archived rows
    /// included) after `after`, in ID order.
    async fn memory_id_batch(
        &self,
        filters: OwnerFilters<'_>,
        after: Option<&RecordId>,
    ) -> Result<Vec<RecordId>> {
        let (mut clauses, binds) = filters.clauses();
        if after.is_some() {
            clauses.push("id > $after");
        }
        if clauses.is_empty() {
            clauses.push("true");
        }
        let sql = format!(
            "SELECT VALUE id FROM memory WHERE {} ORDER BY id ASC LIMIT {DELETE_BATCH}",
            clauses.join(" AND ")
        );
        let db = self.live_db()?;
        let mut query = db.query(sql);
        for bind in binds {
            query = query.bind(bind);
        }
        if let Some(after) = after {
            query = query.bind(("after", after.clone()));
        }
        Ok(query.await?.take(0)?)
    }
}
