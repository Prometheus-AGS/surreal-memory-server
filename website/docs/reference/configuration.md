---
id: configuration
title: Configuration
sidebar_position: 2
---

# Configuration

All configuration is environment-driven.

## Database

| Variable | Default | Notes |
|---|---|---|
| `SURREAL_MODE` | `embedded` | `embedded` or `server` |
| `SURREAL_PATH` | `./data/memory.db` | Embedded RocksDB path |
| `SURREAL_ENDPOINT` | — | e.g. `ws://127.0.0.1:8000` |
| `SURREAL_USERNAME` / `SURREAL_PASSWORD` | — | Server mode credentials |
| `SURREAL_NAMESPACE` / `SURREAL_DATABASE` | — | Target namespace/database |
| `SURREAL_EMBEDDED_MAX_INFLIGHT` | `16` | Matches RocksDB stripe count |
| `SURREAL_QUERY_TIMEOUT_MS` | `10000` | Per-query ceiling |

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

## Executor

| Variable | Default | Covers |
|---|---|---|
| `SURREAL_EXECUTOR_STARTUP_MS` | `300000` | spawn → readiness |
| `SURREAL_EXECUTOR_WATCHDOG_MS` | `30000` | per-request progress |

## Server

| Variable | Default |
|---|---|
| `API_PORT` | `3001` |
| `MCP_STDIO` | `false` |
| `RUST_LOG` | `info` |
