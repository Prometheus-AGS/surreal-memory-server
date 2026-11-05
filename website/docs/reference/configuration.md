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

## Authentication and tenancy

`AUTH_MODE=local` (the default) serves an unauthenticated API for the trusted
local client and is **refused at startup** unless `API_HOST` is a loopback
address. `AUTH_MODE=jwt` is the shared-deployment mode: every route except
`/health` and `/ready` requires a Gate-issued bearer token, and startup fails if
any setting below is missing.

| Variable | Default | Notes |
|---|---|---|
| `AUTH_MODE` | `local` | `local` or `jwt` |
| `AUTH_JWT_ISSUER` | – | required; must match the token `iss` |
| `AUTH_JWT_AUDIENCE` | – | required; must match the token `aud` |
| `AUTH_JWT_ALGORITHM` | – | required; `HS256`, `RS256`, `ES256` or `EdDSA` |
| `AUTH_JWT_KEY` / `AUTH_JWT_KEY_FILE` | – | one is required; an HS256 secret (at least 32 bytes) or a public-key PEM |
| `AUTH_JWT_MAX_AGE_SECS` | `300` | a token older than this is refused, so a revoked grant stops working within this window |

Token claims: `iss`, `aud`, `sub`, `tenant` (`[a-z0-9_]{1,48}`), `iat`, `exp`,
optional `ops` (`"operator"` allows retry, reject and re-key of durable
operations) and optional `user_id` (restricts the credential to one user's
memories, and to the `/api/v1/memory` and `/api/v1/search` routes only).

**Isolation model: one database per tenant.** The tenant comes only from the
verified token. In server mode it maps to the namespace
`<SURREAL_NAMESPACE>_<tenant>` (the SurrealDB user needs rights to create
namespaces); in embedded mode to the directory `<SURREAL_PATH>/tenants/<tenant>`.
Operation IDs, receipts, events, stats and replay live in that tenant's database,
so the same client operation ID in two tenants is two independent operations.
MCP sessions (streamable HTTP and legacy SSE) are bound to the `(tenant, sub)`
that opened them. The local stdio MCP transport remains the trusted local client.

## Server

| Variable | Default | Notes |
|---|---|---|
| `API_HOST` | `127.0.0.1` | Bind address. Anything other than loopback requires `AUTH_MODE=jwt` |
| `API_PORT` | `3001` | |
| `MCP_STDIO` | `true` | Set `false` for HTTP-only daemons |
| `RUST_LOG` | `info` | |
| `TTL_INTERVAL_SECS` | `3600` | TTL expiry worker interval |
