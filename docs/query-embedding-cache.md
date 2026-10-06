# Query embedding cache

`SurrealStorage::search_memories` uses a bounded, per-storage-instance cache of
query embeddings. This change hardens its provider/model identity contract; it
is not evidence of a reproduced cross-model leak in the previous per-instance
cache. Search results and persisted memory embeddings are not cached here.

## Identity and configuration

A key contains the provider/model/configuration namespace, embedding dimensions,
and query text normalized to NFC with whitespace collapsed to single spaces.
The namespace is length-framed so a delimiter inside a model name cannot make
another identity's key collide. Normalization and capacity policy are unchanged.

`EmbeddingService::cache_namespace() -> Option<&str>` is backward compatible:
the default is `None`. A provider must report a nonsecret identity covering its
effective model and embedding-affecting configuration. Unknown providers bypass
caching, even if their dimensions match a known model. A lazy provider may
publish its identity once it has identified its worker. The cache freezes the
first known namespace and dimensions; a later mismatch bypasses it. Replacing a
model or configuration requires constructing a new service and cache.

The built-in identities cover:

- OpenAI: model, fixed embedding endpoint, default requested dimensions and
  expected output dimensions; no API key.
- Cohere: model, fixed endpoint, `search_document` input type, float output and
  expected dimensions; no API key.
- Candle: model and configured revision, captured CPU/automatic device policy,
  compiled CUDA/Metal capabilities, F32 BERT inference, masked mean pooling,
  L2 normalization and tokenizer special-token policy.
- FastEmbed: the pinned 4.9.1 default, BGE-small-en-v1.5, using
  `Xenova/bge-small-en-v1.5/onnx/model.onnx`, 512-token maximum, CLS pooling and
  L2 normalization. The no-op adapter reports no identity.
- Supervised execution: the actual worker's advertised namespace plus its
  validated backend, model, revision, dimensions and protocol version. Rust and
  MLX workers send the optional `cache_namespace` readiness field. MLX includes
  its actual settings and fixed mean pooling, normalization and no-layernorm
  configuration. The supervisor freezes its first readiness identity and rejects
  a replacement worker with a different identity. A legacy worker without the
  field remains usable but bypasses caching. Calls before the first readiness
  handshake also bypass caching.

The FastEmbed name above is source-grounded, rather than taken from the older
MiniLM comments in mempalace: the pinned mempalace `FastEmbedder::new_default`
uses `InitOptions::default()`. In fastembed 4.9.1, `src/text_embedding/mod.rs`
selects `BGESmallENV15` and maximum length 512; `init.rs` uses those defaults;
`models/text_embedding.rs` selects the Xenova ONNX artifact; and
`text_embedding/impl.rs` selects CLS pooling. No model selection or dependency
version was changed. `Cargo.lock` retains `quick_cache` 0.6.21.

Identities describe configured models and inference settings. A remote provider
can update the implementation behind a model alias without announcing it; this
cache does not claim to detect such upstream changes or fingerprint model files.
Credentials and private cache-directory paths are excluded from identities.

`SURREAL_MEMORY_QUERY_EMBED_CACHE` sets capacity, defaulting to 512 when unset or
invalid. Zero disables caching. The construction-time
`with_query_embed_cache_capacity` builder replaces the cache and resets its
identity and counters. An unknown namespace bypasses caching even when the
reported configured capacity is nonzero.

## Counters and coalescing

`query_embed_cache_stats()` exposes the existing fields and types:

| Field | Meaning |
| --- | --- |
| `capacity: usize` | Configured maximum entries; zero disables caching. |
| `entries: usize` | Values currently retained in the bounded cache. |
| `hits: u64` | Successful query-embedding reuse without invoking that caller's producer, including a successful coalesced waiter. |
| `misses: u64` | Query-embedding producer attempts begun, including attempts that fail or are cancelled after starting. |

A cacheable producer increments `misses` immediately before awaiting
`EmbeddingService::embed`. A successful reuse increments `hits` only after the
cache returns a value. Failed values are never installed. When capacity is zero
or identity is unknown, each attempted query embedding increments one miss and
zero hits. A search with limit zero returns without attempting an embedding and
changes neither counter.

Concurrent queries for the same cacheable key can share a successful producer:
one miss and one hit for each successful waiter that did not produce. A producer
that fails or is cancelled leaves no value; a waiter can then become the next
producer and count another miss. A waiter cancelled before obtaining a result or
starting its own producer changes neither counter. A producer cancelled after
starting retains its miss.

These counters measure the query cache's calls to the embedding service, not
successful HTTP requests or every lower-level executor command. The supervised
provider can retry a child request internally after protocol/transport loss;
that remains one cache producer invocation. Warmup, planning, writes and other
embedding-service users are outside these counters. A database search can fail
after its embedding was produced or reused, so the embedding counter still
stands. Consequently, `hits + misses` need not equal successful searches.
Snapshots read independent atomic counters and the current cache length; they
are observational values, not a transactionally synchronized total.

## Writes and operation statistics

Create/update paths embed content directly. Duplicate detection reuses that
content embedding through `search_memories_with_embedding`; it does not read,
fill or increment the query cache. The public `MemoryStorage` trait is unchanged.

`GET /api/v2/operations/stats` includes the existing `query_embed_cache` object on
every successful supported-backend response. Its Rust field remains optional for
wire compatibility with older responses. The operation service explicitly
requires `SurrealStorage` before ledger I/O. Unsupported storage propagates
`durable operations require SurrealStorage`; the existing HTTP handler returns
503 with that `error` string instead of silently omitting cache statistics.
This is a backend requirement, not a network/transport failure. Existing ledger
errors also propagate normally.

## Acceptance boundary

The source implementation is not runtime acceptance evidence. The final local
integration boundary for `change-ldd-12-integration-rollout` owns production HTTP
search/stats coverage with isolated real SurrealDB and an actual local executor:
repeat/coalesced queries, capacity zero, worker failure, model/config identities
across restarts, and write/dedup counter invariance. Executor observations must
distinguish cache producer invocations from internal child retries and warmup.
No build, test, formatter or acceptance gate was run while authoring this change.
