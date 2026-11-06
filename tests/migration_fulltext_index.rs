//! Fresh-database migration through the real storage entry point, ending with
//! the full-text index behind the BM25 leg of hybrid search.
//!
//! Every migration, v24 (`memory_fulltext_index`) included, must apply to an
//! empty database, and keyword search must then work. A 3.3.0 server rejects
//! v24: it defines the analyzer and the index in one transaction, and 3.3.0
//! cannot see the uncommitted analyzer ("The analyzer 'memory_text' does not
//! exist"). The certified 4.0.0 server and the embedded engine accept it.
//!
//! Run against a scratch 4.0.0 server, never the shared `:28000`:
//!
//! `TEST_SURREAL_ENDPOINT=ws://127.0.0.1:28100 cargo test --test migration_fulltext_index`
//!
//! Without `TEST_SURREAL_ENDPOINT` the suite runs on an isolated embedded engine.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};
use surreal_memory::{EmbeddingService, Memory, MemoryStorage, SurrealStorage};

const DIMENSIONS: usize = 8;

const ZEBRA: &str = "the zebra crossed the river at dawn";
const BUDGET: &str = "quarterly budget review notes";

/// Each memory sits on its own axis (so they are not merged as near-duplicates)
/// and the query on a third, orthogonal to both: vector similarity cannot tell
/// them apart, so only the BM25 leg can rank the keyword match first.
struct AxisEmbedder;

#[async_trait::async_trait]
impl EmbeddingService for AxisEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let axis = match text {
            ZEBRA => 1,
            BUDGET => 2,
            _ => 3,
        };
        let mut vector = vec![0.0f32; DIMENSIONS];
        vector[axis] = 1.0;
        Ok(vector)
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

async fn fresh_storage() -> SurrealStorage {
    let embedder: Arc<dyn EmbeddingService> = Arc::new(AxisEmbedder);
    match std::env::var("TEST_SURREAL_ENDPOINT") {
        Ok(endpoint) => {
            let run = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let config = SurrealConfig {
                mode: SurrealMode::Server,
                endpoint: Some(endpoint),
                username: Some("root".into()),
                password: Some("root".into()),
                namespace: format!("migration_ft_{run}"),
                database: "main".into(),
                ..Default::default()
            };
            SurrealStorage::new(&config, embedder)
                .await
                .unwrap_or_else(|error| panic!("server storage failed: {error:#}"))
        }
        Err(_) => SurrealStorage::new_mem(embedder)
            .await
            .unwrap_or_else(|error| panic!("embedded storage failed: {error:#}")),
    }
}

#[tokio::test]
async fn fresh_database_migrates_to_a_working_fulltext_index() {
    // Opening storage runs every migration; a rejected one fails here.
    let storage = fresh_storage().await;

    for content in [ZEBRA, BUDGET] {
        storage
            .add_memory(Memory::new(
                content.to_owned(),
                Some("migration-test".to_owned()),
                None,
                None,
                Vec::new(),
            ))
            .await
            .expect("store memory");
    }

    // The BM25 leg queries `content @@ $query`, which needs the full-text index.
    let hits = storage
        .hybrid_search_memories(
            "zebra",
            Some("migration-test"),
            None,
            None,
            None,
            5,
            0.0,
            1.0,
        )
        .await
        .expect("hybrid search");
    assert_eq!(
        hits.first().map(|memory| memory.content.as_str()),
        Some(ZEBRA),
        "BM25 leg did not rank the keyword match first: {:?}",
        hits.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
}
