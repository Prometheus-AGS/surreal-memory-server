//! Evidence-backed compression: publish protocol (issue #54, PR 1).
//!
//! Embedded-mode tests against `SurrealStorage::new_mem`. They prove the
//! pipeline publishes atomically, keeps (archives) its sources, records
//! claim-to-evidence links, and changes nothing when anything fails.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use surreal_memory::compression::FaultPoint;
use surreal_memory::embeddings::EmbeddingService;
use surreal_memory::{Memory, MemoryStorage, SurrealStorage, TaskStream, profile_for};
use surrealdb::types::RecordIdKey;

const DIMENSIONS: usize = 8;
const FILLER: &str =
    "The team talked through the schedule, the lunch order and the weekly calendar in detail. ";

/// Deterministic embedder: distinct texts get unrelated unit vectors, so the
/// 0.92 dedup never merges the seeded memories.
struct HashEmbedder;

#[async_trait::async_trait]
impl EmbeddingService for HashEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let digest = Sha256::digest(text.as_bytes());
        let raw: Vec<f32> = digest[..DIMENSIONS]
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
        DIMENSIONS
    }
}

async fn storage() -> Arc<SurrealStorage> {
    Arc::new(
        SurrealStorage::new_mem(Arc::new(HashEmbedder))
            .await
            .expect("embedded storage"),
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

/// One rule inside bulk filler. `label` makes the memory distinct, `rule` is
/// the sentence the extractive summarizer should keep.
fn with_rule(label: &str, rule: &str) -> String {
    format!("{FILLER}{FILLER}{rule} {FILLER}{label} noted.")
}

async fn seed(s: &SurrealStorage, stream: &str, user: &str, contents: Vec<String>) {
    let mut ts = TaskStream::new(stream, None, None, Some(user.to_string()));
    ts.auto_summarize = false;
    s.create_task_stream(ts).await.expect("create stream");
    let per_memory = (profile_for("default").summarization_threshold() / 6 + 1) as u32;
    for content in contents {
        let mut memory = Memory::new(
            content,
            Some(user.to_string()),
            None,
            None,
            vec!["user".into()],
        );
        memory.token_count = Some(per_memory);
        s.add_to_task_stream(stream, None, None, memory)
            .await
            .expect("seed memory");
    }
}

fn distinct_rules(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            with_rule(
                &format!("note {i}"),
                &format!("Never skip validation for step {i}."),
            )
        })
        .collect()
}

async fn active_count(s: &SurrealStorage, stream: &str) -> usize {
    s.get_context_for_task(stream, None, None, "gpt-4o", Some(1_000_000))
        .await
        .expect("context")
        .memories
        .len()
}

#[tokio::test]
async fn publishes_a_summary_and_archives_instead_of_deleting_sources() {
    let s = storage().await;
    seed(&s, "pub", "u1", distinct_rules(8)).await;
    let before = s.get_task_stream("pub", None, None).await.unwrap().unwrap();
    let all = s
        .get_context_for_task("pub", None, None, "gpt-4o", Some(1_000_000))
        .await
        .unwrap()
        .memories;
    assert_eq!(all.len(), 8);
    let oldest_four: Vec<String> = {
        let mut ordered = all.clone();
        ordered.sort_by_key(|m| m.created_at);
        ordered.iter().take(4).map(id_of).collect()
    };

    let summary = s
        .auto_summarize_task_stream("pub", Some("u1"), None, "default")
        .await
        .expect("summarize")
        .expect("a summary is published");

    assert!(
        summary
            .content
            .starts_with("[extractive summary of 4 memories]")
    );
    assert!(summary.content.contains("Never skip validation for step"));
    assert!(
        !summary.content.contains("lunch"),
        "irrelevant filler must be dropped"
    );
    assert_eq!(
        summary.user_id.as_deref(),
        Some("u1"),
        "summary keeps the sources' owner"
    );

    // Sources are archived, not deleted: still readable by id, no longer current.
    for id in &oldest_four {
        assert!(
            s.get_memory(id).await.unwrap().is_some(),
            "{id} must still exist"
        );
    }
    assert_eq!(
        active_count(&s, "pub").await,
        5,
        "4 newer sources + the summary"
    );

    // Counters were fixed in the same transaction, by exactly what changed.
    let after = s.get_task_stream("pub", None, None).await.unwrap().unwrap();
    let per_memory = u64::from(profile_for("default").summarization_threshold() as u32 / 6 + 1);
    assert_eq!(after.summary_count, before.summary_count + 1);
    assert_eq!(
        after.total_tokens,
        before.total_tokens - 4 * per_memory + u64::from(summary.token_count.unwrap())
    );
}

#[tokio::test]
async fn every_claim_resolves_to_source_ids_and_versions() {
    let s = storage().await;
    seed(&s, "claims", "u1", distinct_rules(8)).await;
    let summary = s
        .auto_summarize_task_stream("claims", Some("u1"), None, "default")
        .await
        .unwrap()
        .unwrap();

    let claims = s.claims_for_summary(&id_of(&summary)).await.unwrap();
    assert_eq!(claims.len(), 4, "one claim per distinct rule");
    for claim in &claims {
        assert_eq!(claim.kind, "constraint");
        assert!(!claim.evidence.is_empty());
        for evidence in &claim.evidence {
            assert_eq!(evidence.source_version, 1);
            assert!(s.get_memory(&evidence.source_id).await.unwrap().is_some());
            assert!(claim.text.contains(evidence.quote.trim_end_matches('.')));
        }
        // One independent root each: not promoted on a single observation.
        assert_eq!(claim.status, "candidate");
        assert_eq!(
            claim.reason.as_deref(),
            Some("insufficient_independent_evidence")
        );
    }
}

#[tokio::test]
async fn a_rule_seen_in_distinct_sources_is_promoted_once() {
    let s = storage().await;
    let rule = "Never commit secrets to the repository.";
    let contents = (0..8)
        .map(|i| with_rule(&format!("remark {i}"), rule))
        .collect();
    seed(&s, "promote", "u1", contents).await;
    let summary = s
        .auto_summarize_task_stream("promote", Some("u1"), None, "default")
        .await
        .unwrap()
        .unwrap();

    let claims = s.claims_for_summary(&id_of(&summary)).await.unwrap();
    assert_eq!(claims.len(), 1, "equivalent claims are merged");
    let claim = &claims[0];
    assert_eq!(claim.status, "promoted");
    assert_eq!(claim.corroboration, 4);
    assert_eq!(claim.evidence.len(), 4);
    assert_eq!(claim.evidence_strength.as_deref(), Some("corroborated:4"));
}

#[tokio::test]
async fn content_with_nothing_to_keep_is_deferred_and_untouched() {
    let s = storage().await;
    let contents = (0..8)
        .map(|i| format!("Step {i} of the walkthrough."))
        .collect();
    seed(&s, "bare", "u1", contents).await;
    let before = s
        .get_task_stream("bare", None, None)
        .await
        .unwrap()
        .unwrap();

    let result = s
        .auto_summarize_task_stream("bare", Some("u1"), None, "default")
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "no extractable claim means a deferred no-op"
    );
    let after = s
        .get_task_stream("bare", None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.total_tokens, before.total_tokens);
    assert_eq!(after.summary_count, 0);
    assert_eq!(active_count(&s, "bare").await, 8);
}

#[tokio::test]
async fn a_failure_at_any_stage_changes_nothing_and_a_retry_succeeds_once() {
    for point in [
        FaultPoint::Generation,
        FaultPoint::Validation,
        FaultPoint::Insertion,
        FaultPoint::Activation,
    ] {
        let s = storage().await;
        let name = format!("fault-{point:?}").to_lowercase();
        seed(&s, &name, "u1", distinct_rules(8)).await;
        let before = s.get_task_stream(&name, None, None).await.unwrap().unwrap();

        s.compression().inject_fault(point);
        let failed = s
            .auto_summarize_task_stream(&name, Some("u1"), None, "default")
            .await;
        if point == FaultPoint::Activation {
            // The failure happens inside the transaction, which rolls back and
            // reports a clean deferral rather than an error.
            assert!(
                matches!(failed, Ok(None)),
                "{point:?}: expected a rolled-back deferral"
            );
        } else {
            assert!(
                failed.is_err(),
                "{point:?}: the injected fault must surface"
            );
        }

        let unchanged = s.get_task_stream(&name, None, None).await.unwrap().unwrap();
        assert_eq!(
            unchanged.total_tokens, before.total_tokens,
            "{point:?}: counter changed"
        );
        assert_eq!(unchanged.summary_count, 0, "{point:?}: partial activation");
        assert_eq!(
            active_count(&s, &name).await,
            8,
            "{point:?}: a source was lost or hidden"
        );

        let summary = s
            .auto_summarize_task_stream(&name, Some("u1"), None, "default")
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{point:?}: the retry must publish"));
        let done = s.get_task_stream(&name, None, None).await.unwrap().unwrap();
        assert_eq!(done.summary_count, 1, "{point:?}: duplicate derivation");
        assert_eq!(
            s.claims_for_summary(&id_of(&summary)).await.unwrap().len(),
            4
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_duplicate_requests_publish_exactly_one_summary() {
    let s = storage().await;
    seed(&s, "race", "u1", distinct_rules(8)).await;

    let calls: Vec<_> = (0..4)
        .map(|_| {
            let s = Arc::clone(&s);
            tokio::spawn(async move {
                s.auto_summarize_task_stream("race", Some("u1"), None, "default")
                    .await
            })
        })
        .collect();
    for call in calls {
        call.await.expect("task").expect("no call may error");
    }

    let after = s
        .get_task_stream("race", None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.summary_count, 1,
        "exactly one derivation is activated"
    );
    assert_eq!(
        active_count(&s, "race").await,
        5,
        "no source lost, no duplicate summary"
    );
}

#[tokio::test]
async fn compress_memories_never_merges_across_owners_or_widens_scope() {
    let s = storage().await;
    for (user, label) in [("u1", "a"), ("u1", "b"), ("u2", "c"), ("u2", "d")] {
        let memory = Memory::new(
            with_rule(label, &format!("Never share {label} credentials.")),
            Some(user.to_string()),
            None,
            None,
            vec!["user".into()],
        );
        s.add_memory(memory).await.expect("seed");
    }

    // Unfiltered: sources belong to two users, so there is no safe owner.
    let mixed = s.compress_memories(None, None, None, 0).await.unwrap();
    assert!(
        mixed.is_none(),
        "mixed owners must defer, not produce an unscoped summary"
    );

    let own = s
        .compress_memories(Some("u1"), None, None, 0)
        .await
        .unwrap()
        .expect("u1's memories compress");
    assert_eq!(
        own.user_id.as_deref(),
        Some("u1"),
        "the summary is never wider than its sources"
    );
    let claims = s.claims_for_summary(&id_of(&own)).await.unwrap();
    assert!(claims.iter().all(|c| c.text.contains("credentials")));
    assert!(
        claims
            .iter()
            .all(|c| !c.text.contains('c') || c.text.contains("credentials")),
        "no content from u2 leaks into u1's summary"
    );
}

#[tokio::test]
async fn archived_sources_leave_search_but_the_summary_stays_findable() {
    let s = storage().await;
    let contents = distinct_rules(8);
    seed(&s, "find", "u1", contents.clone()).await;
    let ctx = s
        .get_context_for_task("find", None, None, "gpt-4o", Some(1_000_000))
        .await
        .unwrap();
    let oldest = {
        let mut ordered = ctx.memories.clone();
        ordered.sort_by_key(|m| m.created_at);
        ordered[0].clone()
    };
    let hit_before = s
        .search_memories(&oldest.content, Some("u1"), None, None, None, 10)
        .await
        .unwrap();
    assert!(hit_before.iter().any(|m| id_of(m) == id_of(&oldest)));

    s.auto_summarize_task_stream("find", Some("u1"), None, "default")
        .await
        .unwrap()
        .unwrap();

    let hit_after = s
        .search_memories(&oldest.content, Some("u1"), None, None, None, 10)
        .await
        .unwrap();
    assert!(
        hit_after.iter().all(|m| id_of(m) != id_of(&oldest)),
        "an archived source must not be returned as current knowledge"
    );
    assert!(
        s.get_memory(&id_of(&oldest)).await.unwrap().is_some(),
        "but it is retained as evidence"
    );
}

#[tokio::test]
async fn legacy_memories_carry_no_invented_trust() {
    let s = storage().await;
    let stored = s
        .add_memory(Memory::new(
            "An old fact.",
            Some("u1".to_string()),
            None,
            None,
            Vec::<String>::new(),
        ))
        .await
        .unwrap();
    assert!(
        s.claims_for_summary(&id_of(&stored))
            .await
            .unwrap()
            .is_empty()
    );
}
