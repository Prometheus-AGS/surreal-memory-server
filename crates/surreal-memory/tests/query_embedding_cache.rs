//! The bounded query-embedding cache in `SurrealStorage` (tlh-05): repeated and
//! concurrent identical searches embed once, different text embeds again,
//! capacity 0 disables the cache, and the write path neither reads nor fills it.
//!
//! Runs a real embedded SurrealStorage; only the embedding service is a
//! counting stand-in, so each count is the number of embedding calls the
//! storage actually made.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use surreal_memory::embeddings::EmbeddingService;
use surreal_memory::{Memory, MemoryStorage, SurrealStorage};

const DIMENSIONS: usize = 8;
const USER: &str = "cache-user";

struct CountingEmbedder {
    calls: AtomicUsize,
    delay: Duration,
}

impl CountingEmbedder {
    fn new(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            delay,
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl EmbeddingService for CountingEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        let mut vector = vec![0.0f32; DIMENSIONS];
        // Each stored text gets its own axis so no write is merged as a
        // near-duplicate of another; query-only texts share the last axis.
        let axis = match text {
            "seed memory" => 0,
            "shared text" => 1,
            "write only text" => 2,
            _ => 3,
        };
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

/// Storage with one seeded memory. Returns the embedder and the call count
/// after seeding, so assertions measure search-time embeddings only.
async fn fixture(
    capacity: usize,
    delay: Duration,
) -> (Arc<SurrealStorage>, Arc<CountingEmbedder>, usize) {
    let embedder = CountingEmbedder::new(delay);
    let service: Arc<dyn EmbeddingService> = Arc::clone(&embedder) as _;
    let storage = SurrealStorage::new_mem(service)
        .await
        .expect("embedded storage")
        .with_query_embed_cache_capacity(capacity);
    let storage = Arc::new(storage);
    storage
        .add_memory(Memory::new(
            "seed memory".to_owned(),
            Some(USER.to_owned()),
            None,
            None,
            vec![],
        ))
        .await
        .expect("seed memory");
    let baseline = embedder.calls();
    (storage, embedder, baseline)
}

async fn search(storage: &SurrealStorage, query: &str) {
    storage
        .search_memories(query, Some(USER), None, None, None, 3)
        .await
        .expect("search");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repeated_identical_searches_embed_once() {
    let (storage, embedder, baseline) = fixture(512, Duration::ZERO).await;
    for _ in 0..4 {
        search(&storage, "what changed in the release").await;
    }
    assert_eq!(embedder.calls() - baseline, 1);
    let stats = storage.query_embed_cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.entries), (3, 1, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whitespace_variants_of_one_query_share_an_entry() {
    let (storage, embedder, baseline) = fixture(512, Duration::ZERO).await;
    search(&storage, "release   notes").await;
    search(&storage, "  release notes\n").await;
    assert_eq!(embedder.calls() - baseline, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_identical_searches_embed_once() {
    let (storage, embedder, baseline) = fixture(512, Duration::from_millis(150)).await;
    let query = "concurrent recall query";
    tokio::join!(
        search(&storage, query),
        search(&storage, query),
        search(&storage, query),
        search(&storage, query),
    );
    assert_eq!(embedder.calls() - baseline, 1);
    let stats = storage.query_embed_cache_stats();
    assert_eq!((stats.hits, stats.misses), (3, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn different_text_embeds_again() {
    let (storage, embedder, baseline) = fixture(512, Duration::ZERO).await;
    search(&storage, "first query").await;
    search(&storage, "second query").await;
    search(&storage, "first query").await;
    assert_eq!(embedder.calls() - baseline, 2);
    let stats = storage.query_embed_cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.entries), (1, 2, 2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capacity_zero_embeds_every_time() {
    let (storage, embedder, baseline) = fixture(0, Duration::ZERO).await;
    for _ in 0..4 {
        search(&storage, "uncached query").await;
    }
    assert_eq!(embedder.calls() - baseline, 4);
    let stats = storage.query_embed_cache_stats();
    assert_eq!((stats.capacity, stats.hits, stats.entries), (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_memory_neither_reads_nor_fills_the_cache() {
    let (storage, embedder, baseline) = fixture(512, Duration::ZERO).await;
    search(&storage, "shared text").await;
    let before = storage.query_embed_cache_stats();
    let calls_before = embedder.calls();

    // Same text as the cached query: a write must still embed it itself and
    // must not touch the counters or the entry count.
    storage
        .add_memory(Memory::new(
            "shared text".to_owned(),
            Some(USER.to_owned()),
            Some("writer".to_owned()),
            None,
            vec![],
        ))
        .await
        .expect("add memory");
    assert_eq!(embedder.calls() - calls_before, 1, "write embeds directly");
    assert_eq!(storage.query_embed_cache_stats(), before);

    // A brand-new write text is not cached for later searches either.
    storage
        .add_memory(Memory::new(
            "write only text".to_owned(),
            Some(USER.to_owned()),
            Some("writer".to_owned()),
            None,
            vec![],
        ))
        .await
        .expect("add memory");
    let calls_after_write = embedder.calls();
    search(&storage, "write only text").await;
    assert_eq!(embedder.calls() - calls_after_write, 1);
    assert!(embedder.calls() > baseline);
}
