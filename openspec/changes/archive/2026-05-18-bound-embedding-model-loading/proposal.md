# Bound Embedding Model Loading

## Why

Assessment findings **C-1**, **H-3**, **H-4**, **L-1**, **M-2** form one
root-cause cluster: embedding model loads are **unbounded, unobserved, and block
the async runtime**.

`CandleEmbeddings::ensure_loaded` (`embeddings/candle.rs`) loads the BERT model
lazily on the first embed call. On a cold cache it performs three sequential
HuggingFace Hub downloads with no timeout and no progress. The synchronous,
CPU-heavy load (`Tokenizer::from_file`, `VarBuilder::from_*`, `BertModel::load`)
and the subsequent forward pass run **directly on a tokio worker thread with no
`spawn_blocking`**, starving other tasks. Palace's `FastEmbedService::new()`
(`palace/embedding.rs`) has the same cold-download hang. The startup log claims
"Embedding service ready" when nothing is actually loaded ("will verify on first
use"), so a client cannot tell a healthy server from one about to hang 4 minutes.

This was the embedding-dependent arm of the `create_entity` 4-minute hang that
forced a client to abandon the tool.

## What Changes

- Move synchronous model load + inference off the tokio runtime via
  `tokio::task::spawn_blocking` — in both `crates/surreal-memory/src/embeddings/candle.rs`
  and the binary copy `src/embeddings/candle.rs`.
- Add a bounded timeout and a typed error to `download_model` so a cold-cache or
  network failure surfaces as a clear, fast error instead of an open-ended hang.
- Apply the same bounded-load contract to Palace `FastEmbedService::new()`
  (`palace/embedding.rs`, `palace/context.rs`).
- Add an optional, config-gated startup **warmup embed** so first-write latency
  is paid at boot, not on the user's first call.
- Make `/health` and the embedding-readiness log distinguish "process up" from
  "embedding model loaded and verified".
- L-1: parallelize the three HuggingFace file downloads in `download_model`.

## Non-goals

- Switching embedding providers or model formats.
- Pre-bundling model weights into the Docker image (separate ops concern).
- The connection/retry stall (covered by `eliminate-write-path-stalls`).
- MCP progress notifications (covered by `add-mcp-progress-reporting`).

## Impact

- No persisted struct field added → no migration expected. If a warmup config
  field is introduced it lives in config structs only (not SurrealDB-persisted);
  confirm no `DEFINE FIELD` is required.
- `spawn_blocking` changes the concurrency shape of `embed`; the `OnceCell` +
  `Mutex` invariants in `CandleEmbeddings` must be preserved.
- Affects every embedding-dependent write (`create_entity`, `add_memory`, etc.)
  and all Palace operations.
