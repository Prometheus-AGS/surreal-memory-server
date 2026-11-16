//! Bounded, resumable memory enumeration (issue #62).
//!
//! Every check runs against embedded storage; the server-mode twins (ignored by
//! default, they need `TEST_SURREAL_ENDPOINT` pointing at a scratch server, never
//! the shared :28000 instance) run the same functions. All data is synthetic.

use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, TimeZone, Utc};
use sha2::{Digest, Sha256};
use surreal_memory::compression::{CompressionRuntime, FaultPoint, FixedClock};
use surreal_memory::embeddings::EmbeddingService;
use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};
use surreal_memory::{
    MAX_MEMORY_PAGE_SIZE, MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS, MEMORY_PAGE_BYTE_BUDGET, Memory,
    MemoryPage, MemoryPageRequest, MemoryStorage, SurrealStorage,
};
use surrealdb::types::{Datetime, RecordId, RecordIdKey};
use surrealdb_types::SurrealValue;

const DIM: usize = 64;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
}

/// A stable pseudo-random unit vector per text, so distinct synthetic
/// memories never trip the 0.92 near-duplicate merge.
struct HashEmbedder {
    dim: usize,
}

#[async_trait::async_trait]
impl EmbeddingService for HashEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut raw = Vec::with_capacity(self.dim);
        let mut block = 0u32;
        while raw.len() < self.dim {
            let mut hasher = Sha256::new();
            hasher.update(block.to_le_bytes());
            hasher.update(text.as_bytes());
            raw.extend(hasher.finalize().iter().map(|b| f32::from(*b) - 127.5));
            block += 1;
        }
        raw.truncate(self.dim);
        let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
        Ok(raw.into_iter().map(|v| v / norm).collect())
    }

    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(self.embed(&text).await?);
        }
        Ok(out)
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

fn runtime() -> CompressionRuntime {
    CompressionRuntime::default().with_clock(Arc::new(FixedClock(t0())))
}

async fn embedded_with(dim: usize) -> Arc<SurrealStorage> {
    Arc::new(
        SurrealStorage::new_mem(Arc::new(HashEmbedder { dim }))
            .await
            .expect("embedded storage")
            .with_compression(runtime()),
    )
}

async fn embedded() -> Arc<SurrealStorage> {
    embedded_with(DIM).await
}

async fn server_with(dim: usize) -> Arc<SurrealStorage> {
    let env = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
    let config = SurrealConfig {
        mode: SurrealMode::Server,
        endpoint: Some(env("TEST_SURREAL_ENDPOINT", "ws://127.0.0.1:28100")),
        username: Some(env("TEST_SURREAL_USERNAME", "root")),
        password: Some(env("TEST_SURREAL_PASSWORD", "root")),
        namespace: format!("enumeration_{}", uuid::Uuid::new_v4().simple()),
        database: "main".to_string(),
        ..Default::default()
    };
    Arc::new(
        SurrealStorage::new(&config, Arc::new(HashEmbedder { dim }))
            .await
            .expect("server storage")
            .with_compression(runtime()),
    )
}

async fn server() -> Arc<SurrealStorage> {
    server_with(DIM).await
}

fn id_of(memory: &Memory) -> String {
    let id = memory.id.as_ref().expect("stored memory has an id");
    let key = match &id.key {
        RecordIdKey::String(value) => value.clone(),
        other => format!("{other:?}"),
    };
    format!("{}:{}", id.table.as_str(), key)
}

fn rid(id: &str) -> RecordId {
    RecordId::parse_simple(id).expect("record id")
}

async fn add(s: &SurrealStorage, user: &str, content: String) -> Memory {
    let memory = Memory::new(
        content,
        Some(user.to_string()),
        None,
        None,
        vec![format!("cat{}", user.len())],
    );
    s.add_memory(memory).await.expect("add memory")
}

/// `n` distinct synthetic memories for `user`; returns their IDs.
async fn seed(s: &SurrealStorage, user: &str, n: usize) -> Vec<String> {
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let content = format!("synthetic memory {i} of {user} {}", uuid::Uuid::new_v4());
        ids.push(id_of(&add(s, user, content).await));
    }
    ids
}

fn request(user: &str, limit: Option<usize>, cursor: Option<String>) -> MemoryPageRequest {
    MemoryPageRequest {
        user_id: Some(user.to_string()),
        limit,
        cursor,
        ..MemoryPageRequest::default()
    }
}

async fn page(s: &SurrealStorage, req: MemoryPageRequest) -> MemoryPage {
    s.list_memories_page(req).await.expect("page")
}

/// Follow cursors to the end; returns IDs in the order received and the pages.
async fn enumerate(s: &SurrealStorage, base: MemoryPageRequest) -> (Vec<String>, Vec<MemoryPage>) {
    let mut req = base;
    let mut ids = Vec::new();
    let mut pages = Vec::new();
    loop {
        let p = page(s, req.clone()).await;
        ids.extend(p.memories.iter().map(id_of));
        let next = p.next_cursor.clone();
        assert_eq!(p.has_more, next.is_some(), "has_more matches the cursor");
        pages.push(p);
        match next {
            Some(cursor) => req.cursor = Some(cursor),
            None => return (ids, pages),
        }
    }
}

fn assert_unique(ids: &[String]) {
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "no memory is returned twice");
}

fn encoded_len(memories: &[Memory]) -> usize {
    serde_json::to_vec(memories).expect("json").len()
}

async fn physical_ids(s: &SurrealStorage, user: &str) -> Vec<RecordId> {
    s.db()
        .expect("db")
        .query("SELECT VALUE id FROM memory WHERE user_id = $u")
        .bind(("u", user.to_string()))
        .await
        .expect("query")
        .take(0)
        .expect("ids")
}

async fn run(s: &SurrealStorage, sql: &str, id: &str) {
    s.db()
        .expect("db")
        .query(sql)
        .bind(("rid", rid(id)))
        .await
        .expect("query")
        .check()
        .expect("statement");
}

#[derive(Debug, SurrealValue)]
struct ShapeRow {
    id: RecordId,
    embedding: Option<Vec<f32>>,
}

// ── Shared checks (run on embedded and server storage) ───────────────────────

/// Task 1.1: `OMIT embedding` with a keyset predicate drops vectors and
/// returns rows in ascending ID order on SurrealDB 4.0.
async fn check_query_shape(s: Arc<SurrealStorage>) {
    let mut ids = seed(&s, "shape", 9).await;
    ids.sort();
    let rows: Vec<ShapeRow> = s
        .db()
        .expect("db")
        .query(
            "SELECT * OMIT embedding FROM memory WHERE user_id = $u AND id > $after \
             ORDER BY id ASC LIMIT $n",
        )
        .bind(("u", "shape".to_string()))
        .bind(("after", rid(&ids[2])))
        .bind(("n", 4))
        .await
        .expect("query")
        .take(0)
        .expect("rows");
    let got: Vec<String> = rows
        .iter()
        .map(|r| match &r.id.key {
            RecordIdKey::String(k) => format!("memory:{k}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(got, ids[3..7].to_vec(), "ascending IDs after the cursor");
    assert!(rows.iter().all(|r| r.embedding.is_none()), "no embeddings");
}

/// Task 1.3: print the plans, and assert the owner-filtered keyset query is an
/// index scan with no sort.
async fn check_explain(s: Arc<SurrealStorage>, mode: &str) {
    let ids = seed(&s, "plan", 3).await;
    let live =
        "(status IS NONE OR status = 'active') AND (valid_until IS NONE OR valid_until >= $now)";
    let filtered = format!(
        "SELECT * OMIT embedding FROM memory WHERE user_id = $u AND {live} AND id > $after \
         ORDER BY id ASC LIMIT 51 EXPLAIN"
    );
    let unfiltered = format!(
        "SELECT * OMIT embedding FROM memory WHERE {live} AND id > $after \
         ORDER BY id ASC LIMIT 51 EXPLAIN"
    );
    let mut plans = Vec::new();
    for (name, sql) in [("filtered", filtered), ("unfiltered", unfiltered)] {
        let plan: Vec<surrealdb_types::Value> = s
            .db()
            .expect("db")
            .query(sql)
            .bind(("u", "plan".to_string()))
            .bind(("now", Datetime::from(t0())))
            .bind(("after", rid(&ids[0])))
            .await
            .expect("explain")
            .take(0)
            .expect("plan");
        let text = format!("{plan:?}");
        println!("EXPLAIN [{mode}] {name}: {text}");
        plans.push(text);
    }
    assert!(
        plans[0].contains("IndexScan"),
        "filtered query uses an index"
    );
    assert!(!plans[0].contains("Sort"), "filtered query does not sort");
    assert!(!plans[1].contains("Sort"), "unfiltered query does not sort");
}

/// Task 2.3: limits, cursor validation, empty scope and last page.
async fn check_validation(s: Arc<SurrealStorage>) {
    seed(&s, "many", 55).await;
    let first = page(&s, request("many", None, None)).await;
    assert_eq!(first.memories.len(), 50, "default page size");
    assert!(first.has_more && first.next_cursor.is_some());

    let invalid = |e: anyhow::Error, needle: &str| {
        let text = e.to_string();
        assert!(text.contains("invalid"), "{text}");
        assert!(text.contains(needle), "{text} names {needle}");
    };
    invalid(
        s.list_memories_page(request("many", Some(0), None))
            .await
            .unwrap_err(),
        "limit 0",
    );
    invalid(
        s.list_memories_page(request("many", Some(MAX_MEMORY_PAGE_SIZE + 1), None))
            .await
            .unwrap_err(),
        &MAX_MEMORY_PAGE_SIZE.to_string(),
    );
    let mut with_embeddings = request("many", Some(MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS + 1), None);
    with_embeddings.include_embeddings = true;
    invalid(
        s.list_memories_page(with_embeddings).await.unwrap_err(),
        &MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS.to_string(),
    );
    for bad in ["not-a-cursor", "entity:abc", "memory:", "memory:a b", ""] {
        invalid(
            s.list_memories_page(request("many", None, Some(bad.to_string())))
                .await
                .unwrap_err(),
            "cursor",
        );
    }

    let empty = page(&s, request("nobody", None, None)).await;
    assert!(empty.memories.is_empty() && !empty.has_more && empty.next_cursor.is_none());

    seed(&s, "few", 3).await;
    let last = page(&s, request("few", None, None)).await;
    assert_eq!(last.memories.len(), 3);
    assert!(
        !last.has_more && last.next_cursor.is_none(),
        "last page has no cursor"
    );
}

/// Task 2.4: exact-once enumeration, resume, concurrent insert and delete,
/// expired and archived rows hidden.
async fn check_enumeration(s: Arc<SurrealStorage>) {
    let stored = seed(&s, "enum", 23).await;
    let (ids, pages) = enumerate(&s, request("enum", Some(5), None)).await;
    assert_eq!(pages.len(), 5);
    assert_unique(&ids);
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "pages are in ascending ID order");
    let expected: HashSet<String> = stored.iter().cloned().collect();
    assert_eq!(ids.iter().cloned().collect::<HashSet<_>>(), expected);

    // Resume from a saved cursor with a fresh request.
    let first = page(&s, request("enum", Some(5), None)).await;
    let (rest, _) = enumerate(&s, request("enum", Some(5), first.next_cursor.clone())).await;
    let mut resumed: Vec<String> = first.memories.iter().map(id_of).collect();
    resumed.extend(rest);
    assert_eq!(resumed, ids, "resuming continues with none repeated");

    // Concurrent insert between pages.
    let first = page(&s, request("enum", Some(5), None)).await;
    let added = seed(&s, "enum", 4).await;
    let (rest, _) = enumerate(&s, request("enum", Some(5), first.next_cursor.clone())).await;
    let mut seen: Vec<String> = first.memories.iter().map(id_of).collect();
    seen.extend(rest);
    assert_unique(&seen);
    for id in &stored {
        assert_eq!(
            seen.iter().filter(|s| *s == id).count(),
            1,
            "pre-existing once"
        );
    }
    for id in &added {
        assert!(
            seen.iter().filter(|s| *s == id).count() <= 1,
            "new at most once"
        );
    }

    // Concurrent delete of a memory not yet returned.
    let first = page(&s, request("enum", Some(5), None)).await;
    let cursor = first.next_cursor.clone().expect("more pages");
    let victim = ids
        .iter()
        .find(|id| **id > cursor)
        .expect("a later id")
        .clone();
    s.delete_memory(&victim).await.expect("delete");
    let (rest, _) = enumerate(&s, request("enum", Some(5), Some(cursor))).await;
    assert!(!rest.contains(&victim), "deleted memory does not resurface");

    // Expired and archived memories stay hidden.
    let mut expired = Memory::new(
        "synthetic expired".to_string(),
        Some("enum".to_string()),
        None,
        None,
        Vec::<String>::new(),
    );
    expired.valid_until = Some(Datetime::from(t0() - Duration::days(1)));
    let expired = id_of(&s.add_memory(expired).await.expect("add expired"));
    let archived = stored[3].clone();
    run(&s, "UPDATE $rid SET status = 'summarized'", &archived).await;
    let (ids, _) = enumerate(&s, request("enum", Some(7), None)).await;
    assert!(!ids.contains(&expired), "expired hidden");
    assert!(!ids.contains(&archived), "archived hidden");
}

/// Task 2.5: embeddings are omitted unless requested.
async fn check_projection(s: Arc<SurrealStorage>) {
    seed(&s, "proj", 6).await;
    let plain = page(&s, request("proj", None, None)).await;
    assert_eq!(plain.memories.len(), 6);
    assert!(plain.memories.iter().all(|m| m.embedding.is_none()));

    let mut req = request("proj", Some(MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS), None);
    req.include_embeddings = true;
    let full = page(&s, req).await;
    assert_eq!(full.memories.len(), 6);
    for memory in &full.memories {
        let stored = s
            .get_memory(&id_of(memory))
            .await
            .expect("get")
            .expect("present");
        assert_eq!(memory.embedding.as_ref().map(Vec::len), Some(DIM));
        assert_eq!(memory.embedding, stored.embedding, "stored embedding");
    }
}

fn big_content(tag: &str, bytes: usize) -> String {
    let mut content = format!("synthetic large {tag} ");
    while content.len() < bytes {
        content.push_str("filler ");
    }
    content
}

/// Task 2.6: the byte budget shortens pages; an oversized memory is returned
/// alone and enumeration moves past it.
async fn check_byte_budget(s: Arc<SurrealStorage>) {
    for i in 0..6 {
        add(&s, "big", big_content(&format!("b{i}"), 300 * 1024)).await;
    }
    let first = page(&s, request("big", None, None)).await;
    assert!(first.memories.len() < 6, "budget shortens the page");
    assert!(first.has_more && first.next_cursor.is_some());
    assert!(encoded_len(&first.memories) <= MEMORY_PAGE_BYTE_BUDGET);

    let mut stored = seed(&s, "huge", 4).await;
    stored.push(id_of(
        &add(
            &s,
            "huge",
            big_content("oversized", MEMORY_PAGE_BYTE_BUDGET + 256 * 1024),
        )
        .await,
    ));
    let (ids, pages) = enumerate(&s, request("huge", None, None)).await;
    assert_unique(&ids);
    assert_eq!(
        ids.iter().cloned().collect::<HashSet<_>>(),
        stored.iter().cloned().collect::<HashSet<_>>()
    );
    for p in &pages {
        let oversized = p
            .memories
            .iter()
            .any(|m| m.content.len() > MEMORY_PAGE_BYTE_BUDGET);
        if oversized {
            assert_eq!(p.memories.len(), 1, "an oversized memory is returned alone");
        } else {
            assert!(encoded_len(&p.memories) <= MEMORY_PAGE_BYTE_BUDGET);
        }
    }
}

/// Task 2.7: owner filters apply on every page, whatever the cursor.
async fn check_scope(s: Arc<SurrealStorage>) {
    let mine: HashSet<String> = seed(&s, "alice", 7).await.into_iter().collect();
    seed(&s, "bob", 6).await;
    let (ids, _) = enumerate(&s, request("alice", Some(3), None)).await;
    assert_eq!(ids.iter().cloned().collect::<HashSet<_>>(), mine);

    let bob_cursor = page(&s, request("bob", Some(2), None))
        .await
        .next_cursor
        .expect("bob has more");
    let (ids, _) = enumerate(&s, request("alice", Some(3), Some(bob_cursor))).await;
    assert!(
        ids.iter().all(|id| mine.contains(id)),
        "only alice's memories"
    );
}

/// Task 6.1: the maximum limit over large memories is bounded at the database
/// (shortened pages), and enumeration still returns every memory once.
async fn check_large_maximum_page(s: Arc<SurrealStorage>, count: usize, bytes: usize) {
    let mut stored = Vec::with_capacity(count);
    for i in 0..count {
        let content = big_content(&format!("large{i} {}", uuid::Uuid::new_v4()), bytes);
        stored.push(id_of(&add(&s, "large", content).await));
    }
    let (ids, pages) = enumerate(&s, request("large", Some(MAX_MEMORY_PAGE_SIZE), None)).await;
    assert_unique(&ids);
    assert_eq!(
        ids.iter().cloned().collect::<HashSet<_>>(),
        stored.iter().cloned().collect::<HashSet<_>>()
    );
    let counts: Vec<usize> = pages.iter().map(|p| p.memories.len()).collect();
    assert!(
        counts.iter().all(|n| *n < count),
        "pages are shortened: {counts:?}"
    );
    for p in &pages {
        assert!(encoded_len(&p.memories) <= MEMORY_PAGE_BYTE_BUDGET || p.memories.len() == 1);
    }
    println!(
        "LARGE count={count} bytes_each={bytes} limit={MAX_MEMORY_PAGE_SIZE} pages={} page_counts={counts:?}",
        pages.len()
    );
}

/// Task 6.2: a record key that does not round-trip through the cursor (a
/// UUID-typed key comes back as a string key) never repeats a page.
async fn check_cursor_must_advance(s: Arc<SurrealStorage>) {
    for _ in 0..2 {
        s.db()
            .expect("db")
            .query(
                "CREATE type::record('memory', rand::uuid::v4()) SET content = 'synthetic uuid key', \
                 user_id = 'uuidkey', scope = 'user', memory_type = 'semantic', categories = [], \
                 importance = 0.5, access_count = 0, version = 1, \
                 created_at = time::now(), updated_at = time::now()",
            )
            .await
            .expect("query")
            .check()
            .expect("create");
    }
    let first = page(&s, request("uuidkey", Some(1), None)).await;
    assert_eq!(first.memories.len(), 1);
    let cursor = first.next_cursor.clone().expect("a second row remains");
    let first_id = id_of(&first.memories[0]);
    match s
        .list_memories_page(request("uuidkey", Some(1), Some(cursor)))
        .await
    {
        Err(error) => {
            let text = format!("{error:#}");
            assert!(text.contains("did not advance"), "{text}");
            println!("CURSOR uuid key: page did not advance; rejected");
        }
        Ok(next) => {
            assert_eq!(next.memories.len(), 1, "the second row is not skipped");
            assert!(
                next.memories.iter().all(|m| id_of(m) != first_id),
                "a page is never repeated"
            );
            println!("CURSOR uuid key: page advanced; no repeat");
        }
    }
    // The legacy loop must terminate either way (error or complete list).
    let _ = s.get_all_memories(Some("uuidkey"), None, None).await;
}

/// Task 3.1/3.2: the legacy complete listing is complete and never partial.
async fn check_legacy_listing(s: Arc<SurrealStorage>) {
    let stored = seed(&s, "legacy", 60).await;
    let all = s
        .get_all_memories(Some("legacy"), None, None)
        .await
        .expect("all");
    let ids: Vec<String> = all.iter().map(id_of).collect();
    assert_unique(&ids);
    assert_eq!(
        ids.iter().cloned().collect::<HashSet<_>>(),
        stored.iter().cloned().collect::<HashSet<_>>()
    );
    assert!(
        all.iter().all(|m| m.embedding.is_some()),
        "legacy keeps embeddings"
    );

    // Corrupt a row that falls on a later page: the call fails instead of
    // returning the pages before it.
    let mut sorted = stored.clone();
    sorted.sort();
    run(&s, "UPDATE $rid SET scope = 'not-a-scope'", &sorted[40]).await;
    assert!(
        s.get_all_memories(Some("legacy"), None, None)
            .await
            .is_err(),
        "a failing page is an error, not a partial list"
    );
}

/// Task 3.3: bulk delete over several batches, physical view, scope-limited.
async fn check_bulk_delete(s: Arc<SurrealStorage>) {
    seed(&s, "wipe", 205).await;
    for i in 0..2 {
        let mut memory = Memory::new(
            format!("synthetic expired wipe {i}"),
            Some("wipe".to_string()),
            None,
            None,
            Vec::<String>::new(),
        );
        memory.valid_until = Some(Datetime::from(t0() - Duration::days(1)));
        s.add_memory(memory).await.expect("add expired");
    }
    seed(&s, "keep", 5).await;
    let deleted = s
        .delete_all_memories(Some("wipe"), None, None)
        .await
        .expect("delete all");
    assert_eq!(deleted, 207, "expired memories are deleted and counted");
    assert!(physical_ids(&s, "wipe").await.is_empty(), "scope emptied");
    assert_eq!(
        physical_ids(&s, "keep").await.len(),
        5,
        "other scope untouched"
    );
}

/// Task 3.4: a failure reports the progress made and a rerun finishes.
async fn check_bulk_delete_failure(s: Arc<SurrealStorage>) {
    let mut ids = seed(&s, "partial", 10).await;
    ids.sort();
    // A row that cannot be decoded makes its own deletion fail after three.
    run(&s, "UPDATE $rid SET scope = 'not-a-scope'", &ids[3]).await;
    let error = s
        .delete_all_memories(Some("partial"), None, None)
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("after deleting 3 memories"), "{text}");
    assert!(text.contains("delete_all_memories"), "{text}");
    run(&s, "UPDATE $rid SET scope = 'user'", &ids[3]).await;
    let rest = s
        .delete_all_memories(Some("partial"), None, None)
        .await
        .expect("rerun");
    assert_eq!(rest, 7, "the rerun deletes the remainder");
    assert!(physical_ids(&s, "partial").await.is_empty());

    // The injected fault point fails the next deletion.
    seed(&s, "faulted", 4).await;
    s.compression().inject_fault(FaultPoint::BulkDelete);
    let text = s
        .delete_all_memories(Some("faulted"), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(text.contains("after deleting 0 memories"), "{text}");
    assert_eq!(
        s.delete_all_memories(Some("faulted"), None, None)
            .await
            .expect("rerun"),
        4
    );
}

// ── Embedded mode ────────────────────────────────────────────────────────────

#[tokio::test]
async fn omit_embedding_keyset_query_returns_ascending_rows_without_vectors() {
    check_query_shape(embedded().await).await;
}

#[tokio::test]
async fn keyset_query_plan_uses_the_owner_index_without_sorting() {
    check_explain(embedded().await, "embedded").await;
}

#[tokio::test]
async fn page_limits_cursors_empty_scope_and_last_page() {
    check_validation(embedded().await).await;
}

#[tokio::test]
async fn following_cursors_enumerates_every_memory_exactly_once() {
    check_enumeration(embedded().await).await;
}

#[tokio::test]
async fn embeddings_are_omitted_unless_requested() {
    check_projection(embedded().await).await;
}

#[tokio::test]
async fn the_byte_budget_shortens_pages_and_oversized_memories_still_progress() {
    check_byte_budget(embedded().await).await;
}

#[tokio::test]
async fn owner_filters_apply_to_every_page_whatever_the_cursor() {
    check_scope(embedded().await).await;
}

#[tokio::test]
async fn the_legacy_complete_listing_is_complete_and_never_partial() {
    check_legacy_listing(embedded().await).await;
}

#[tokio::test]
async fn bulk_delete_covers_several_batches_and_expired_rows_only_in_scope() {
    check_bulk_delete(embedded().await).await;
}

#[tokio::test]
async fn interrupted_bulk_delete_reports_progress_and_a_rerun_finishes() {
    check_bulk_delete_failure(embedded().await).await;
}

#[tokio::test]
async fn the_maximum_limit_over_large_memories_is_bounded_and_complete() {
    check_large_maximum_page(embedded().await, 30, 100 * 1024).await;
}

#[tokio::test]
async fn a_cursor_that_does_not_advance_never_repeats_a_page() {
    check_cursor_must_advance(embedded().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_a_cursor_that_does_not_advance_never_repeats_a_page() {
    check_cursor_must_advance(server().await).await;
}

/// Task 6.1 on the original failure's scale: 200 memories of 400 KiB, the
/// maximum limit, one scope. Before the size pre-pass this asked the server
/// for about 80 MiB in one response.
#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_the_maximum_limit_over_400_kib_memories_is_bounded_and_complete() {
    check_large_maximum_page(server().await, 200, 400 * 1024).await;
}

/// Writer that keeps everything the log subscriber prints.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Task 2.8: a database failure names the request's shape, not its data.
#[tokio::test]
async fn a_failed_listing_names_its_bounds_without_private_data() {
    const CONTENT: &str = "synthetic-private-content-7f3a";
    const USER: &str = "synthetic-private-user-91c2";
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let s = embedded().await;
    add(&s, USER, CONTENT.to_string()).await;
    s.db()
        .expect("db")
        .query("REMOVE TABLE memory")
        .await
        .expect("query")
        .check()
        .expect("remove table");

    let error = s
        .list_memories_page(request(USER, None, None))
        .await
        .unwrap_err();
    let chain = format!("{error:#}");
    let log = String::from_utf8(captured.0.lock().expect("log").clone()).expect("utf8");
    for text in [&chain, &log] {
        assert!(text.contains("list_memories_page"), "{text}");
        assert!(text.contains("50"), "page size in {text}");
        assert!(text.contains("include_embeddings"), "{text}");
        assert!(text.contains("user_id"), "{text}");
        assert!(!text.contains(CONTENT), "no content in {text}");
        assert!(!text.contains(USER), "no scope value in {text}");
        assert!(!text.contains("root"), "no credentials in {text}");
    }
    assert!(
        chain.contains("page_size=50, include_embeddings=false, filters: user_id=set, agent_id=unset, session_id=unset"),
        "{chain}"
    );
}

/// Task 2.2: a backend relying on the default fails closed.
#[tokio::test]
async fn a_backend_without_paging_fails_closed() {
    let error = Unpaged
        .list_memories_page(MemoryPageRequest::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not supported"), "{error}");
}

// ── Server mode (needs TEST_SURREAL_ENDPOINT; run with --ignored) ────────────

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_omit_embedding_keyset_query_returns_ascending_rows_without_vectors() {
    check_query_shape(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_keyset_query_plan_uses_the_owner_index_without_sorting() {
    check_explain(server().await, "server").await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_page_limits_cursors_empty_scope_and_last_page() {
    check_validation(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_following_cursors_enumerates_every_memory_exactly_once() {
    check_enumeration(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_embeddings_are_omitted_unless_requested() {
    check_projection(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_the_byte_budget_shortens_pages_and_oversized_memories_still_progress() {
    check_byte_budget(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_owner_filters_apply_to_every_page_whatever_the_cursor() {
    check_scope(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_the_legacy_complete_listing_is_complete_and_never_partial() {
    check_legacy_listing(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_bulk_delete_covers_several_batches_and_expired_rows_only_in_scope() {
    check_bulk_delete(server().await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_interrupted_bulk_delete_reports_progress_and_a_rerun_finishes() {
    check_bulk_delete_failure(server().await).await;
}

// ── Measurement (task 5.1 / 5.2; run with --ignored --nocapture) ─────────────

const MEASURE_USER: &str = "measure";
const MEASURE_DIM: usize = 1536;

/// Seed `count` memories of about `bytes` content each.
async fn seed_sized(s: &SurrealStorage, count: usize, bytes: usize) {
    for i in 0..count {
        add(
            s,
            MEASURE_USER,
            big_content(&format!("m{i} {}", uuid::Uuid::new_v4()), bytes),
        )
        .await;
    }
}

/// Enumerate with and without embeddings and print the largest page.
async fn measure_pages(s: &SurrealStorage, mode: &str, count: usize, bytes: usize) {
    let total = count * bytes;
    for include_embeddings in [false, true] {
        let limit = if include_embeddings {
            MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS
        } else {
            MAX_MEMORY_PAGE_SIZE
        };
        let mut req = request(MEASURE_USER, Some(limit), None);
        req.include_embeddings = include_embeddings;
        let (ids, pages) = enumerate(s, req).await;
        assert_eq!(ids.len(), count);
        let sizes: Vec<usize> = pages.iter().map(|p| encoded_len(&p.memories)).collect();
        let largest = sizes.iter().copied().max().unwrap_or(0);
        let counts: Vec<usize> = pages.iter().map(|p| p.memories.len()).collect();
        println!(
            "MEASURE [{mode}] memories={count} content_bytes_each={bytes} content_total={total} \
             include_embeddings={include_embeddings} limit={limit} pages={} \
             largest_page_bytes={largest} page_counts={counts:?}",
            pages.len()
        );
        for (p, size) in pages.iter().zip(&sizes) {
            assert!(
                *size <= MEMORY_PAGE_BYTE_BUDGET || p.memories.len() == 1,
                "page within the budget"
            );
        }
    }
}

#[tokio::test]
#[ignore = "measurement; run with --ignored --nocapture"]
async fn measure_page_sizes_embedded() {
    let s = embedded_with(MEASURE_DIM).await;
    seed_sized(&s, 400, 32 * 1024).await;
    measure_pages(&s, "embedded", 400, 32 * 1024).await;
}

#[tokio::test]
#[ignore = "measurement; needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn measure_page_sizes_server() {
    let s = server_with(MEASURE_DIM).await;
    seed_sized(&s, 400, 32 * 1024).await;
    measure_pages(&s, "server", 400, 32 * 1024).await;
}

/// Task 5.2: the pre-change unbounded query against a scope larger than the
/// SDK's 64 MiB WebSocket message limit. Scratch server only. Reports what
/// happened; it does not assert a failure.
#[tokio::test]
#[ignore = "baseline; needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn baseline_unbounded_query_server() {
    const COUNT: usize = 200;
    const BYTES: usize = 400 * 1024;
    let s = server_with(MEASURE_DIM).await;
    seed_sized(&s, COUNT, BYTES).await;
    let started = std::time::Instant::now();
    let outcome: Result<Vec<Memory>, _> = s
        .db()
        .expect("db")
        .query("SELECT * FROM memory WHERE user_id = $u")
        .bind(("u", MEASURE_USER.to_string()))
        .await
        .and_then(|mut response| response.take(0));
    match outcome {
        Ok(rows) => println!(
            "BASELINE unbounded: ok rows={} encoded_json_bytes={} elapsed_ms={}",
            rows.len(),
            encoded_len(&rows),
            started.elapsed().as_millis()
        ),
        Err(error) => println!(
            "BASELINE unbounded: error after {} ms: {error}",
            started.elapsed().as_millis()
        ),
    }
    // The bounded path over the same scope, and whether the shared handle
    // still works afterwards.
    let (ids, _) = enumerate(&s, request(MEASURE_USER, None, None)).await;
    println!("BASELINE paged after unbounded: memories={}", ids.len());
}

// ── A backend that relies on the trait's default `list_memories_page` ────────

struct Unpaged;

macro_rules! unused {
    () => {
        unimplemented!("not used by this test")
    };
}

#[async_trait::async_trait]
impl MemoryStorage for Unpaged {
    async fn create_entity(
        &self,
        _: surreal_memory::Entity,
    ) -> anyhow::Result<surreal_memory::Entity> {
        unused!()
    }
    async fn create_entities(
        &self,
        _: Vec<surreal_memory::Entity>,
    ) -> anyhow::Result<Vec<surreal_memory::Entity>> {
        unused!()
    }
    async fn get_entity(&self, _: &str) -> anyhow::Result<Option<surreal_memory::Entity>> {
        unused!()
    }
    async fn update_entity(
        &self,
        _: surreal_memory::Entity,
    ) -> anyhow::Result<surreal_memory::Entity> {
        unused!()
    }
    async fn delete_entity(&self, _: &str) -> anyhow::Result<()> {
        unused!()
    }
    async fn search_entities(&self, _: &str) -> anyhow::Result<Vec<surreal_memory::Entity>> {
        unused!()
    }
    async fn create_relation(
        &self,
        _: surreal_memory::Relation,
    ) -> anyhow::Result<surreal_memory::Relation> {
        unused!()
    }
    async fn create_relations(
        &self,
        _: Vec<surreal_memory::Relation>,
    ) -> anyhow::Result<Vec<surreal_memory::Relation>> {
        unused!()
    }
    async fn get_relations(&self, _: &str) -> anyhow::Result<Vec<surreal_memory::Relation>> {
        unused!()
    }
    async fn delete_relation(&self, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
        unused!()
    }
    async fn get_graph(&self) -> anyhow::Result<surreal_memory::KnowledgeGraph> {
        unused!()
    }
    async fn add_observations(
        &self,
        _: &str,
        _: Vec<String>,
    ) -> anyhow::Result<surreal_memory::Entity> {
        unused!()
    }
    async fn semantic_search(
        &self,
        _: &str,
        _: usize,
        _: f32,
    ) -> anyhow::Result<Vec<surreal_memory::SemanticSearchResult>> {
        unused!()
    }
    async fn add_memory(&self, _: Memory) -> anyhow::Result<Memory> {
        unused!()
    }
    async fn get_memory(&self, _: &str) -> anyhow::Result<Option<Memory>> {
        unused!()
    }
    async fn update_memory(&self, _: &str, _: String) -> anyhow::Result<Memory> {
        unused!()
    }
    async fn delete_memory(&self, _: &str) -> anyhow::Result<()> {
        unused!()
    }
    async fn delete_all_memories(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<u64> {
        unused!()
    }
    async fn get_all_memories(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Vec<Memory>> {
        unused!()
    }
    async fn search_memories(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&[String]>,
        _: usize,
    ) -> anyhow::Result<Vec<Memory>> {
        unused!()
    }
    async fn get_memory_history(
        &self,
        _: &str,
    ) -> anyhow::Result<Vec<surreal_memory::MemoryHistory>> {
        unused!()
    }
    async fn hybrid_search_memories(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&[String]>,
        _: usize,
        _: f32,
        _: f32,
    ) -> anyhow::Result<Vec<Memory>> {
        unused!()
    }
    async fn rekey_unattributed_agent_id(
        &self,
        _: &str,
        _: Option<&str>,
        _: bool,
    ) -> anyhow::Result<surreal_memory::RekeyReport> {
        unused!()
    }
    async fn compress_memories(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: u32,
    ) -> anyhow::Result<Option<Memory>> {
        unused!()
    }
    async fn add_memories_from_conversation(
        &self,
        _: Vec<serde_json::Value>,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Vec<Memory>> {
        unused!()
    }
    async fn expire_stale_memories(&self) -> anyhow::Result<u64> {
        unused!()
    }
    async fn create_task_stream(
        &self,
        _: surreal_memory::TaskStream,
    ) -> anyhow::Result<surreal_memory::TaskStream> {
        unused!()
    }
    async fn get_task_stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Option<surreal_memory::TaskStream>> {
        unused!()
    }
    async fn add_to_task_stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Memory,
    ) -> anyhow::Result<Memory> {
        unused!()
    }
    async fn get_context_for_task(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
        _: Option<u64>,
    ) -> anyhow::Result<surreal_memory::ContextWindow> {
        unused!()
    }
    async fn list_task_streams(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Vec<surreal_memory::TaskStream>> {
        unused!()
    }
    async fn delete_task_stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<()> {
        unused!()
    }
    async fn archive_task_stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<surreal_memory::TaskStream> {
        unused!()
    }
    async fn pause_task_stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<surreal_memory::TaskStream> {
        unused!()
    }
    async fn add_task_step(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: surreal_memory::TaskStep,
    ) -> anyhow::Result<surreal_memory::TaskStep> {
        unused!()
    }
    async fn update_task_step_status(
        &self,
        _: &str,
        _: surreal_memory::TaskStepStatus,
        _: Option<String>,
        _: Option<String>,
    ) -> anyhow::Result<surreal_memory::TaskStep> {
        unused!()
    }
    async fn get_task_steps(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Vec<surreal_memory::TaskStep>> {
        unused!()
    }
    async fn get_current_step(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Option<surreal_memory::TaskStep>> {
        unused!()
    }
    async fn complete_step(
        &self,
        _: &str,
        _: Option<String>,
    ) -> anyhow::Result<surreal_memory::TaskStep> {
        unused!()
    }
    async fn create_mindmap(
        &self,
        _: surreal_memory::MindMap,
    ) -> anyhow::Result<surreal_memory::MindMap> {
        unused!()
    }
    async fn get_mindmap(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Option<surreal_memory::MindMap>> {
        unused!()
    }
    async fn add_mindmap_node(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: surreal_memory::MindMapNode,
    ) -> anyhow::Result<surreal_memory::MindMap> {
        unused!()
    }
    async fn add_mindmap_edge(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: surreal_memory::MindMapEdge,
    ) -> anyhow::Result<surreal_memory::MindMap> {
        unused!()
    }
    async fn delete_mindmap_node(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
    ) -> anyhow::Result<surreal_memory::MindMap> {
        unused!()
    }
    async fn list_mindmaps(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<Vec<surreal_memory::MindMap>> {
        unused!()
    }
    async fn delete_mindmap(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> anyhow::Result<()> {
        unused!()
    }
    async fn auto_summarize_task_stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
    ) -> anyhow::Result<Option<Memory>> {
        unused!()
    }
    async fn try_update_persona_mindmap(&self, _: &str, _: &Memory) -> anyhow::Result<()> {
        unused!()
    }
    async fn find_path(&self, _: &str, _: &str, _: u8) -> anyhow::Result<Vec<Vec<String>>> {
        unused!()
    }
    async fn expand_neighbors(
        &self,
        _: &str,
        _: u8,
        _: usize,
    ) -> anyhow::Result<surreal_memory::KnowledgeGraph> {
        unused!()
    }
    async fn get_related(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: usize,
    ) -> anyhow::Result<Vec<surreal_memory::Entity>> {
        unused!()
    }
    async fn get_entity_history(
        &self,
        _: &str,
    ) -> anyhow::Result<Vec<surreal_memory::MemoryHistory>> {
        unused!()
    }
    async fn get_graph_at_time(&self, _: &str) -> anyhow::Result<surreal_memory::KnowledgeGraph> {
        unused!()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
