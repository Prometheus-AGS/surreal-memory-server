//! Correctness of `search_memories` now that it runs a scoped KNN query on
//! `memory_embedding_hnsw` instead of ranking the whole scope in Rust
//! (surrealdb-3x-connection-model, c2).
//!
//! Vectors lie in one plane at known angles to the query vector, so each
//! memory's cosine similarity to the query is exactly `cos(angle)` and the
//! expected ranking can be computed here, independently of the database.
//! Memories in one scope are at least 30° apart (similarity <= 0.866), below
//! `add_memory`'s 0.92 duplicate threshold, so only the deliberate duplicate
//! is merged.

use std::collections::HashMap;
use std::sync::Arc;

use surreal_memory::embeddings::EmbeddingService;
use surreal_memory::{Memory, MemoryStorage, SurrealStorage};

const DIMENSIONS: usize = 8;
const QUERY: &str = "query";

/// Unit vector in the plane of the first two axes, `degrees` from the query.
fn at_angle(degrees: f32) -> Vec<f32> {
    let radians = degrees.to_radians();
    let mut vector = vec![0.0f32; DIMENSIONS];
    vector[0] = radians.cos();
    vector[1] = radians.sin();
    vector
}

/// Deterministic embedder: known texts map to fixed vectors.
struct TableEmbedder {
    vectors: HashMap<String, Vec<f32>>,
}

#[async_trait::async_trait]
impl EmbeddingService for TableEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.vectors
            .get(text)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no test vector for {text:?}"))
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

/// (content, user, agent, degrees from the query)
const SEED: &[(&str, &str, &str, f32)] = &[
    ("u1-a1-060", "u1", "a1", 60.0),
    ("u1-a1-000", "u1", "a1", 0.0),
    ("u1-a1-090", "u1", "a1", 90.0),
    ("u1-a1-030", "u1", "a1", 30.0),
    ("u1-a1-150", "u1", "a1", 150.0),
    ("u1-a1-120", "u1", "a1", 120.0),
    // Closer to the query than most of u1/a1, so a scope leak would surface.
    ("u1-a2-015", "u1", "a2", 15.0),
    ("u1-a2-045", "u1", "a2", 45.0),
    ("u1-a2-075", "u1", "a2", 75.0),
    ("u2-a1-005", "u2", "a1", 5.0),
    ("u2-a1-035", "u2", "a1", 35.0),
];

/// Within 3° of `u1-a1-000` (similarity 0.9986): a duplicate in that scope.
const NEAR_DUPLICATE: &str = "near-duplicate";

async fn seeded_storage() -> SurrealStorage {
    let mut vectors: HashMap<String, Vec<f32>> = SEED
        .iter()
        .map(|(content, _, _, degrees)| (content.to_string(), at_angle(*degrees)))
        .collect();
    vectors.insert(QUERY.to_string(), at_angle(0.0));
    vectors.insert(NEAR_DUPLICATE.to_string(), at_angle(3.0));
    let embedder: Arc<dyn EmbeddingService> = Arc::new(TableEmbedder { vectors });

    let storage = SurrealStorage::new_mem(embedder)
        .await
        .expect("embedded SurrealStorage");
    for (content, user, agent, _) in SEED {
        storage
            .add_memory(Memory::new(
                *content,
                Some(user.to_string()),
                Some(agent.to_string()),
                None,
                vec![],
            ))
            .await
            .expect("seed memory");
    }
    storage
}

fn contents(memories: &[Memory]) -> Vec<&str> {
    memories.iter().map(|m| m.content.as_str()).collect()
}

async fn scope_count(storage: &SurrealStorage, user: &str, agent: &str) -> usize {
    storage
        .get_all_memories(Some(user), Some(agent), None)
        .await
        .expect("list scope")
        .len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_memories_matches_exact_ranking_across_scopes() {
    let storage = seeded_storage().await;

    // Every seed is distinct enough to be stored, not merged.
    assert_eq!(scope_count(&storage, "u1", "a1").await, 6);
    assert_eq!(scope_count(&storage, "u1", "a2").await, 3);
    assert_eq!(scope_count(&storage, "u2", "a1").await, 2);

    // Scoped: exact top-3 of u1/a1, with none of the closer u1/a2 or u2/a1 rows.
    let scoped = storage
        .search_memories(QUERY, Some("u1"), Some("a1"), None, None, 3)
        .await
        .expect("scoped search");
    assert_eq!(contents(&scoped), ["u1-a1-000", "u1-a1-030", "u1-a1-060"]);

    // Unscoped: exact top-4 across every scope.
    let unscoped = storage
        .search_memories(QUERY, None, None, None, None, 4)
        .await
        .expect("unscoped search");
    assert_eq!(
        contents(&unscoped),
        ["u1-a1-000", "u2-a1-005", "u1-a2-015", "u1-a1-030"]
    );

    // A limit larger than the scope returns the whole scope, in order.
    let whole_scope = storage
        .search_memories(QUERY, Some("u2"), Some("a1"), None, None, 10)
        .await
        .expect("whole-scope search");
    assert_eq!(contents(&whole_scope), ["u2-a1-005", "u2-a1-035"]);

    // Hybrid search stays inside the requested scope.
    let hybrid = storage
        .hybrid_search_memories(QUERY, Some("u1"), Some("a2"), None, 3, 0.6, 0.4)
        .await
        .expect("hybrid search");
    assert!(!hybrid.is_empty(), "hybrid search returned nothing");
    for memory in &hybrid {
        assert_eq!(memory.user_id.as_deref(), Some("u1"));
        assert_eq!(memory.agent_id.as_deref(), Some("a2"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_memory_merges_a_near_duplicate_only_within_its_scope() {
    let storage = seeded_storage().await;

    // Same scope as `u1-a1-000`: merged into it, scope size unchanged.
    let merged = storage
        .add_memory(Memory::new(
            NEAR_DUPLICATE,
            Some("u1".into()),
            Some("a1".into()),
            None,
            vec![],
        ))
        .await
        .expect("add near-duplicate in scope");
    assert_eq!(merged.content, NEAR_DUPLICATE);
    assert_eq!(scope_count(&storage, "u1", "a1").await, 6);
    let scope = storage
        .get_all_memories(Some("u1"), Some("a1"), None)
        .await
        .expect("list scope");
    assert!(
        scope.iter().all(|m| m.content != "u1-a1-000"),
        "the near-duplicate should have replaced u1-a1-000's content"
    );

    // Different scope: stored as a new memory.
    storage
        .add_memory(Memory::new(
            NEAR_DUPLICATE,
            Some("u2".into()),
            Some("a2".into()),
            None,
            vec![],
        ))
        .await
        .expect("add near-duplicate in another scope");
    assert_eq!(scope_count(&storage, "u2", "a2").await, 1);
}
