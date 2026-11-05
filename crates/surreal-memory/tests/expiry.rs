//! Live-memory expiry and the honest TTL sweep (issue #52).
//!
//! Every check runs against embedded storage; the server-mode twins (ignored by
//! default, they need `TEST_SURREAL_ENDPOINT` pointing at a scratch server)
//! run the same functions. Time comes from an injected `FixedClock`, never from
//! sleeping.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use sha2::{Digest, Sha256};
use surreal_memory::compression::{CompressionRuntime, FaultPoint, FixedClock};
use surreal_memory::embeddings::EmbeddingService;
use surreal_memory::storage::SweepIncomplete;
use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};
use surreal_memory::{Memory, MemoryStorage, SurrealStorage, TaskStream};
use surrealdb::types::{Datetime, RecordIdKey};

const DIM: usize = 8;
const USER: &str = "u1";

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
}

fn at(days: i64) -> Datetime {
    Datetime::from(t0() + Duration::days(days))
}

fn angle(degrees: f32) -> Vec<f32> {
    let radians = degrees.to_radians();
    let mut v = vec![0.0f32; DIM];
    v[0] = radians.cos();
    v[1] = radians.sin();
    v
}

/// Known texts map to fixed vectors; anything else gets a stable pseudo-random
/// unit vector, so unrelated memories never trip the 0.92 dedup.
struct TableEmbedder {
    vectors: HashMap<String, Vec<f32>>,
}

#[async_trait::async_trait]
impl EmbeddingService for TableEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        if let Some(v) = self.vectors.get(text) {
            return Ok(v.clone());
        }
        let digest = Sha256::digest(text.as_bytes());
        let raw: Vec<f32> = digest[..DIM]
            .iter()
            .map(|b| f32::from(*b) - 127.5)
            .collect();
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
        DIM
    }
}

/// Contents of the standard fixture, with their angle from the query vector.
/// Five expired rows are *nearer* than the three live ones, so a filter applied
/// after top-k would return nothing for `limit = 3`.
fn expired_content(i: usize) -> String {
    format!("marker marker marker expired {i}")
}
const BOUNDARY: &str = "marker boundary";
const FUTURE: &str = "marker future";
const NEVER: &str = "marker never";

fn embedder() -> Arc<dyn EmbeddingService> {
    let mut vectors = HashMap::new();
    for (i, deg) in [0.0, 25.0, 50.0, 75.0, 100.0].into_iter().enumerate() {
        vectors.insert(expired_content(i), angle(deg));
    }
    vectors.insert(BOUNDARY.to_string(), angle(125.0));
    vectors.insert(FUTURE.to_string(), angle(150.0));
    vectors.insert(NEVER.to_string(), angle(175.0));
    // The query texts sit at 0 degrees.
    vectors.insert("query-zero".to_string(), angle(0.0));
    vectors.insert("marker".to_string(), angle(0.0));
    vectors.insert("dup-expired".to_string(), angle(0.0));
    vectors.insert("dup-new".to_string(), angle(0.0));
    Arc::new(TableEmbedder { vectors })
}

fn runtime(clock: DateTime<Utc>) -> CompressionRuntime {
    CompressionRuntime::default().with_clock(Arc::new(FixedClock(clock)))
}

async fn embedded(clock: DateTime<Utc>) -> Arc<SurrealStorage> {
    Arc::new(
        SurrealStorage::new_mem(embedder())
            .await
            .expect("embedded storage")
            .with_compression(runtime(clock)),
    )
}

async fn server(clock: DateTime<Utc>) -> Arc<SurrealStorage> {
    let env = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
    let config = SurrealConfig {
        mode: SurrealMode::Server,
        endpoint: Some(env("TEST_SURREAL_ENDPOINT", "ws://127.0.0.1:28100")),
        username: Some(env("TEST_SURREAL_USERNAME", "root")),
        password: Some(env("TEST_SURREAL_PASSWORD", "root")),
        namespace: format!("expiry_{}", uuid::Uuid::new_v4().simple()),
        database: "main".to_string(),
        ..Default::default()
    };
    Arc::new(
        SurrealStorage::new(&config, embedder())
            .await
            .expect("server storage")
            .with_compression(runtime(clock)),
    )
}

fn id_of(memory: &Memory) -> String {
    let id = memory.id.as_ref().expect("stored memory has an id");
    let key = match &id.key {
        RecordIdKey::String(value) => value.clone(),
        other => format!("{other:?}"),
    };
    format!("{}:{}", id.table.as_str(), key)
}

async fn add(s: &SurrealStorage, content: &str, valid_until: Option<Datetime>) -> Memory {
    let mut memory = Memory::new(
        content.to_string(),
        Some(USER.to_string()),
        None,
        None,
        Vec::<String>::new(),
    );
    memory.valid_until = valid_until;
    s.add_memory(memory).await.expect("add memory")
}

struct Fixture {
    expired: Vec<String>,
    boundary: String,
    future: String,
    never: String,
}

impl Fixture {
    fn live(&self) -> HashSet<String> {
        [&self.boundary, &self.future, &self.never]
            .into_iter()
            .cloned()
            .collect()
    }
}

async fn seed(s: &SurrealStorage) -> Fixture {
    let mut expired = Vec::new();
    for i in 0..5 {
        expired.push(id_of(&add(s, &expired_content(i), Some(at(-1))).await));
    }
    Fixture {
        expired,
        // valid_until == the cutoff: still live.
        boundary: id_of(&add(s, BOUNDARY, Some(Datetime::from(t0()))).await),
        future: id_of(&add(s, FUTURE, Some(at(1))).await),
        never: id_of(&add(s, NEVER, None).await),
    }
}

fn ids(memories: &[Memory]) -> Vec<String> {
    memories.iter().map(id_of).collect()
}

async fn deleted_rows(s: &SurrealStorage, id: &str) -> usize {
    s.get_memory_history(id)
        .await
        .expect("history")
        .iter()
        .filter(|h| h.change_type == "deleted")
        .count()
}

// ── Shared checks (run on embedded and server storage) ───────────────────────

async fn check_live_view(s: Arc<SurrealStorage>) {
    let f = seed(&s).await;

    // By id: expired hidden, the rest visible. No sweep has run.
    for id in &f.expired {
        assert!(
            s.get_memory(id).await.unwrap().is_none(),
            "{id} must be hidden"
        );
        assert!(
            s.get_memory_including_expired(id).await.unwrap().is_some(),
            "{id} is still physically present for cleanup and audit"
        );
    }
    for id in f.live() {
        assert!(
            s.get_memory(&id).await.unwrap().is_some(),
            "{id} must stay visible"
        );
    }

    // List.
    let listed: HashSet<String> = ids(&s.get_all_memories(Some(USER), None, None).await.unwrap())
        .into_iter()
        .collect();
    assert_eq!(listed, f.live());

    // KNN: five expired rows are nearer than the three live ones. The live rows
    // must still fill the limit, which is only possible if expiry is filtered
    // before the top-k rather than after it.
    let hits = s
        .search_memories("query-zero", Some(USER), None, None, None, 3)
        .await
        .unwrap();
    assert_eq!(
        ids(&hits),
        vec![f.boundary.clone(), f.future.clone(), f.never.clone()],
        "live rows fill the limit, nearest first"
    );

    // Hybrid: both legs (vector and BM25) are live-only. The expired rows also
    // repeat the query term, so they outrank live rows on BM25.
    let hybrid = s
        .hybrid_search_memories("marker", Some(USER), None, None, None, 3, 0.5, 0.5)
        .await
        .unwrap();
    assert_eq!(hybrid.len(), 3, "limit is filled by live rows");
    for id in ids(&hybrid) {
        assert!(f.live().contains(&id), "{id} is expired but was returned");
    }
}

async fn check_sweep(s: Arc<SurrealStorage>) {
    let f = seed(&s).await;

    let report = s.expire_stale_memories_report().await.unwrap();
    assert_eq!((report.selected, report.removed, report.failed), (5, 5, 0));

    for id in &f.expired {
        assert!(
            s.get_memory_including_expired(id).await.unwrap().is_none(),
            "{id} removed"
        );
        assert_eq!(deleted_rows(&s, id).await, 1, "{id}: exactly one audit row");
    }
    for id in f.live() {
        assert!(s.get_memory(&id).await.unwrap().is_some(), "{id} untouched");
    }
    assert_eq!(
        s.expire_stale_memories().await.unwrap(),
        0,
        "nothing left to remove"
    );
}

async fn check_failed_deletion(s: Arc<SurrealStorage>) {
    let f = seed(&s).await;

    s.compression().inject_fault(FaultPoint::SweepDelete);
    let error = s
        .expire_stale_memories()
        .await
        .expect_err("a failed deletion must surface");
    let incomplete = error
        .downcast_ref::<SweepIncomplete>()
        .expect("the error carries the partial progress");
    assert_eq!(
        (incomplete.selected, incomplete.removed, incomplete.failed),
        (5, 4, 1),
        "only confirmed deletions are counted"
    );

    // Exactly one expired row survived; it is still hidden from normal reads.
    let mut survivors = Vec::new();
    for id in &f.expired {
        if s.get_memory_including_expired(id).await.unwrap().is_some() {
            survivors.push(id.clone());
        }
        assert!(
            s.get_memory(id).await.unwrap().is_none(),
            "{id} stays hidden"
        );
    }
    assert_eq!(survivors.len(), 1);
    for id in f.live() {
        assert!(s.get_memory(&id).await.unwrap().is_some());
    }

    // The next pass retries it, exactly once.
    assert_eq!(s.expire_stale_memories().await.unwrap(), 1);
    assert!(
        s.get_memory_including_expired(&survivors[0])
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(deleted_rows(&s, &survivors[0]).await, 1);
    assert_eq!(s.expire_stale_memories().await.unwrap(), 0);
}

// ── Embedded ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn expired_memories_are_hidden_from_every_normal_read_without_a_sweep() {
    check_live_view(embedded(t0()).await).await;
}

#[tokio::test]
async fn a_sweep_removes_only_expired_rows_keeps_history_and_counts_exactly() {
    check_sweep(embedded(t0()).await).await;
}

#[tokio::test]
async fn a_failed_deletion_is_counted_surfaced_and_retried() {
    check_failed_deletion(embedded(t0()).await).await;
}

#[tokio::test]
async fn task_context_hides_expired_memories() {
    let s = embedded(t0()).await;
    s.create_task_stream(TaskStream::new("ctx", None, None, Some(USER.to_string())))
        .await
        .unwrap();
    let mut stored = Vec::new();
    for (content, valid_until) in [
        ("context expired", Some(at(-1))),
        ("context future", Some(at(1))),
        ("context never", None),
    ] {
        let mut m = Memory::new(
            content,
            Some(USER.to_string()),
            None,
            None,
            Vec::<String>::new(),
        );
        m.valid_until = valid_until;
        stored.push(id_of(
            &s.add_to_task_stream("ctx", None, None, m).await.unwrap(),
        ));
    }

    let ctx = s
        .get_context_for_task("ctx", None, None, "gpt-4o", Some(1_000_000))
        .await
        .unwrap();
    let in_context: HashSet<String> = ids(&ctx.memories).into_iter().collect();
    assert!(
        !in_context.contains(&stored[0]),
        "the expired memory is not in context"
    );
    assert!(in_context.contains(&stored[1]) && in_context.contains(&stored[2]));
}

#[tokio::test]
async fn a_valid_until_equal_to_the_cutoff_is_live_and_one_tick_later_it_is_expired() {
    for (clock, expect_live) in [(t0(), true), (t0() + Duration::microseconds(1), false)] {
        let s = embedded(clock).await;
        let stored = add(&s, BOUNDARY, Some(Datetime::from(t0()))).await;
        let visible = s.get_memory(&id_of(&stored)).await.unwrap().is_some();
        assert_eq!(visible, expect_live, "cutoff {clock}");
    }
}

#[tokio::test]
async fn a_near_duplicate_write_does_not_merge_into_an_expired_memory() {
    let s = embedded(t0()).await;
    let expired = add(&s, "dup-expired", Some(at(-1))).await;

    // Same vector (cosine 1.0, above the 0.92 dedup threshold).
    let fresh = add(&s, "dup-new", None).await;

    assert_ne!(
        id_of(&fresh),
        id_of(&expired),
        "a new row, not a merge into the expired one"
    );
    let history = s.get_memory_history(&id_of(&expired)).await.unwrap();
    assert!(
        history.iter().all(|h| h.change_type == "created"),
        "the expired row was never updated"
    );
    let found = s
        .search_memories("query-zero", Some(USER), None, None, None, 5)
        .await
        .unwrap();
    assert_eq!(ids(&found), vec![id_of(&fresh)]);
}

#[tokio::test]
async fn an_expired_memory_cannot_be_updated_or_renewed() {
    let s = embedded(t0()).await;
    let expired = add(&s, "dup-expired", Some(at(-1))).await;

    assert!(
        s.update_memory(&id_of(&expired), "renewed".into())
            .await
            .is_err()
    );

    let history = s.get_memory_history(&id_of(&expired)).await.unwrap();
    assert!(history.iter().all(|h| h.change_type != "updated"));
    let physical = s
        .get_memory_including_expired(&id_of(&expired))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(physical.content, "dup-expired", "content untouched");
}

#[tokio::test]
async fn compression_never_copies_expired_content_into_a_live_memory() {
    let s = embedded(t0()).await;
    let filler =
        "The team reviewed the calendar, the lunch order and the room bookings in detail. ";
    let body = |rule: &str| format!("{filler}{filler}{rule} {filler}");
    for rule in [
        "Never skip validation for step one.",
        "Never skip validation for step two.",
    ] {
        let mut m = Memory::new(
            body(rule),
            Some("u2".to_string()),
            None,
            None,
            vec!["user".to_string()],
        );
        m.valid_until = None;
        s.add_memory(m).await.unwrap();
    }
    let mut stale = Memory::new(
        body("Never trust EXPIRED-MARKER credentials."),
        Some("u2".to_string()),
        None,
        None,
        vec!["user".to_string()],
    );
    stale.valid_until = Some(at(-1));
    s.add_memory(stale).await.unwrap();

    let summary = s
        .compress_memories(Some("u2"), None, None, 0)
        .await
        .unwrap()
        .expect("the live memories compress");
    assert!(
        !summary.content.contains("EXPIRED-MARKER"),
        "{}",
        summary.content
    );
}

// ── Server mode (needs TEST_SURREAL_ENDPOINT; run with --ignored) ────────────

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_expired_memories_are_hidden_from_every_normal_read() {
    check_live_view(server(t0()).await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_sweep_removes_only_expired_rows() {
    check_sweep(server(t0()).await).await;
}

#[tokio::test]
#[ignore = "needs a scratch SurrealDB server (TEST_SURREAL_ENDPOINT)"]
async fn server_failed_deletion_is_counted_surfaced_and_retried() {
    check_failed_deletion(server(t0()).await).await;
}
