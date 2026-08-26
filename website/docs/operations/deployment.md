---
id: deployment
title: Deployment
sidebar_position: 1
---

# Deployment

## Building

```bash
cargo build --release --no-default-features \
  --features embedded,metal,local-embeddings,palace
```

Feature selection matters — omitting `palace` silently removes 7 MCP tools from
the running server.

| Feature | Effect |
|---|---|
| `embedded` | In-process SurrealDB over RocksDB |
| `server-only` | External SurrealDB, no embedded engine |
| `local-embeddings` | Candle-based local embedding |
| `metal` / `cuda` | GPU backend (implies `local-embeddings`) |
| `palace` | Memory Palace tools and 384d space |

On Apple Silicon, `./build.sh` runs the quality gate then builds with Metal.

## Quality gate

```bash
./scripts/quality-check.sh
```

Runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and the test suite.

:::warning Test isolation
Run integration tests against a **dedicated** SurrealDB, not one shared with
live services. Sharing an instance makes tests compete with production traffic
for the same RocksDB, producing intermittent failures that look like code bugs.
Set `TEST_SURREAL_ENDPOINT` to a scratch server.
:::

## Running as a service (launchd)

The service runs as a LaunchAgent. Points worth getting right:

- `KeepAlive` plus `ThrottleInterval` so a crash loop does not hammer the host.
- `MODEL_CACHE_DIR` must name the HuggingFace home; `hub` is appended when
  resolved.
- `StandardErrorPath` — the executor child inherits stderr, so its logs land in
  the same file.

Reload after changing the plist:

```bash
launchctl unload ~/Library/LaunchAgents/ai.prometheus.surreal-memory-native.plist
launchctl load ~/Library/LaunchAgents/ai.prometheus.surreal-memory-native.plist
```

The server handles `SIGTERM`, so a managed stop logs a clean shutdown rather
than dying silently.

## Health and readiness

| Endpoint | Meaning |
|---|---|
| `GET /health` | Liveness. Static; always 200 if the process serves. |
| `GET /ready` | Readiness. 503 when the database handle is unavailable. |

`/ready` reports per-capability status — `storage`, `ledger`, `model_executor`,
`search_index`, `tokenizer`, `coordinator`. `model_executor: false` with
everything else true means the model has not loaded yet; search will fail until
it does.
