# Tasks — bound-embedding-model-loading

## 1. Bounded, off-runtime Candle load
- [x] 1.1 Wrap synchronous model load in `ensure_loaded` with `spawn_blocking` (`crates/surreal-memory/src/embeddings/candle.rs`)
- [x] 1.2 Binary embedding copies — DEVIATION: `src/embeddings/{candle,cohere,openai}.rs` were dead code (`src/embeddings/mod.rs` only re-exports the library) and were deleted rather than edited
- [x] 1.3 Add a bounded timeout + typed error to `download_model` (`MODEL_DOWNLOAD_TIMEOUT`, 300s, `tokio::time::timeout`)
- [x] 1.4 Parallelize the three HuggingFace file downloads via `tokio::try_join!` (L-1)
- [x] 1.5 Move the forward-pass CPU work off the runtime worker thread (`compute_embedding` in `spawn_blocking`, `blocking_lock`)

## 2. Palace cold-load bound
- [x] 2.1 Apply bounded-load contract to `FastEmbedService::new()` (`FASTEMBED_INIT_TIMEOUT`, 300s)
- [x] 2.2 Verify `PalaceContext::from_storage` surfaces the typed error cleanly (timeout error propagates via `?`)

## 3. Startup warmup
- [x] 3.1 Add an optional, config-gated warmup embed at server start (`EMBEDDING_WARMUP` env, `warmup_embedding`)
- [x] 3.2 Confirm no SurrealDB-persisted field added — `Config.embedding_warmup` is env-derived only; no migration needed

## 4. Health observability
- [x] 4.1 Make `/health` distinguish process-up vs embedding-model-ready (via `EmbeddingService::is_ready()`)
- [x] 4.2 Correct the startup readiness log so it does not claim "ready" prematurely

## 5. Verification
- [x] 5.1 Test: unresolvable model returns a typed error within the bound (`warmup_with_unresolvable_model_fails_bounded`)
- [x] 5.2 Off-runtime load verified by design (`spawn_blocking`); `new_does_not_load_model_eagerly` covers lazy-load contract
- [x] 5.3 fmt + clippy + lib/bin tests clean. DEVIATION: full `cargo check --tests` blocked by pre-existing unrelated error at `tests/contract_alignment_test.rs:312` (deferred follow-up)
- [x] 5.4 QA gate: rust-reviewer — first pass BLOCK (1 CRITICAL, 2 HIGH); refined; re-review APPROVED
