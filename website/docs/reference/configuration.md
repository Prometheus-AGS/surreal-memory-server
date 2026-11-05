---
id: configuration
title: Configuration
sidebar_position: 2
---

# Configuration

All configuration is environment-driven. The server is built and tested
against SurrealDB **4.0.0** (SDK pin `=4.0.0`, server image `v4.0.0`).

## Database

| Variable | Default | Notes |
|---|---|---|
| `SURREAL_MODE` | `embedded` | `embedded` or `server` |
| `SURREAL_PATH` | `./data/memory.db` | Embedded RocksDB path |
| `SURREAL_ENDPOINT` | — | e.g. `ws://127.0.0.1:28000` |
| `SURREAL_USERNAME` / `SURREAL_PASSWORD` | — | Server mode credentials |
| `SURREAL_AUTH_LEVEL` | `root` | `root`, `namespace` or `database` |
| `SURREAL_NAMESPACE` / `SURREAL_DATABASE` | `memory` / `mcp` | Target namespace/database |
| `SURREAL_EMBEDDED_MAX_INFLIGHT` | `16` | Matches RocksDB stripe count |
| `SURREAL_QUERY_TIMEOUT_MS` | `10000` | Per-query ceiling (also the operation-ledger deadline) |
| `SURREAL_OPERATION_DEADLINE_MS` | `30000` | Ceiling for one retried storage operation |

## Retry

| Variable | Default |
|---|---|
| `SURREAL_MAX_CONNECT_RETRIES` | `10` |
| `SURREAL_MAX_OPERATION_RETRIES` | `3` |
| `SURREAL_BASE_RETRY_DELAY_MS` | `100` |
| `SURREAL_MAX_RETRY_DELAY_MS` | `5000` |
| `SURREAL_RETRY_JITTER_FACTOR` | `0.25` |

## Embeddings

| Variable | Default | Notes |
|---|---|---|
| `EMBEDDING_PROVIDER` | `local` | `local`, `openai`, `cohere`, `fast` |
| `LOCAL_EMBEDDING_MODEL` | `BAAI/bge-small-en-v1.5` | 384 dimensions |
| `MODEL_CACHE_DIR` | platform cache | HF *home*; `hub` is appended |
| `MODEL_DOWNLOAD_TIMEOUT_SECS` | `600` | hf-hub ships no timeout of its own |
| `EMBEDDING_WARMUP` | `true` | Set `false` for purely lazy loading |
| `LOCAL_EMBEDDING_BACKEND` | `candle` | `candle` or `mlx` (Apple Silicon only) |
| `LOCAL_EMBEDDING_EXECUTOR` | — | Path to the MLX executor binary |
| `LOCAL_EMBEDDING_MODEL_REVISION` | pinned commit | Hugging Face revision of the model |
| `LOCAL_EMBEDDING_DIMENSIONS` | `384` | Must match the model |
| `LOCAL_EMBEDDING_DEVICE` | `auto` | `auto` or `cpu` (candle) |
| `OPENAI_API_KEY` / `OPENAI_EMBEDDING_MODEL` | — | OpenAI provider |
| `COHERE_API_KEY` / `COHERE_EMBEDDING_MODEL` | — | Cohere provider |

## Compression (summarizer)

`auto_summarize_task_stream` and `compress_memories` run an evidence-backed
pipeline: a summarizer proposes claims that quote their source text, a
deterministic gate checks every quote and the token budget, and the summary is
published atomically. The sources are **archived, not deleted**, and stay
readable by id as evidence. If nothing can be kept safely, the call returns no
summary and changes nothing.

By default the summarizer is **extractive**: it selects sentences, makes no
network calls, and labels its output `extractive`. A model provider is opt-in:
memory text leaves the process, so it needs explicit consent.

| Variable | Default | Meaning |
|---|---|---|
| `SUMMARIZER_PROVIDER` | `extractive` | `extractive` or `openai_compatible` |
| `SUMMARIZER_EGRESS_ALLOWED` | unset | Must be `true` for any network provider; startup fails otherwise |
| `SUMMARIZER_BASE_URL` | none | OpenAI-compatible base URL (for example a local gateway) |
| `SUMMARIZER_MODEL` | none | Model name sent to the provider |
| `SUMMARIZER_API_KEY` | none | Optional bearer token |
| `SUMMARIZER_TIMEOUT_MS` | `20000` | Per-call time limit |
| `SUMMARIZER_MAX_INPUT_TOKENS` | `32000` | Larger inputs are deferred |
| `SUMMARIZER_MAX_OUTPUT_TOKENS` | `1500` | Rendered summary budget |
| `SUMMARIZER_MIN_REDUCTION` | `0.2` | Required shrink (0.2 = at most 80% of the input) |
| `SUMMARIZER_MAX_RETRIES` | `1` | Retries on 429/5xx/timeouts |
| `SUMMARIZER_MAX_TOTAL_TOKENS` | `2000000` | Provider spend cap for the process; further calls fail closed |

A summary never has a wider owner than its sources: sources with different
`user_id`, `agent_id`, `session_id` or scope are not merged. Source text is
sent as untrusted data, and neither source text nor provider responses are
logged.

## Executor

| Variable | Default | Covers |
|---|---|---|
| `SURREAL_EXECUTOR_STARTUP_MS` | `300000` | spawn → readiness |
| `SURREAL_EXECUTOR_WATCHDOG_MS` | `30000` | per-request progress |

## Server

| Variable | Default | Notes |
|---|---|---|
| `API_HOST` | `0.0.0.0` | REST/MCP have no authentication; set `127.0.0.1` for host-local deployments |
| `API_PORT` | `3001` | |
| `MCP_STDIO` | `true` | Set `false` for HTTP-only daemons |
| `RUST_LOG` | `info` | |
| `TTL_INTERVAL_SECS` | `3600` | TTL expiry worker interval |
