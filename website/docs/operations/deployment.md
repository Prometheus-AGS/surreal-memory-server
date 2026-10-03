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

Two LaunchAgents run the stack on macOS. Both plists are rendered from
templates in prometheus-skill-pack (`shared/launchagents/`) by
`scripts/prometheus-services.sh install`; edit the templates, not the
rendered files, or the next install reverts your change.

| Label | Binary | Listens |
|---|---|---|
| `ai.prometheus.surrealdb-native` | `~/.prometheus/bin/surreal` (SurrealDB 3.3.0) | `127.0.0.1:28000` |
| `ai.prometheus.surreal-memory-native` | `/usr/local/bin/surreal-memory-server` | `127.0.0.1:23001` |

The renderer refuses to write a `surrealdb-native` plist for any SurrealDB
version other than the certified one (`SURREALDB_VERSION` in the skill-pack's
`config/defaults.env`), so a stray Homebrew or PATH binary cannot downgrade
the database. The memory-server plist sets `API_HOST=127.0.0.1`: the REST/MCP
endpoints have no authentication.

Points worth getting right:

- `KeepAlive` plus `ThrottleInterval` so a crash loop does not hammer the host.
- `MODEL_CACHE_DIR` must name the HuggingFace home; `hub` is appended when
  resolved.
- `StandardErrorPath` — the executor child inherits stderr, so its logs land in
  the same file.

### Local deploy

```bash
# 1. Build from an up-to-date main
cargo build --release --features embedded,metal,local-embeddings,palace

# 2. Back up the installed binary, then install (keep both copies identical)
ts=$(date +%Y%m%d-%H%M%S)
cp -p /usr/local/bin/surreal-memory-server /usr/local/bin/surreal-memory-server.bak-$ts
install -m 755 target/release/surreal-memory-server /usr/local/bin/surreal-memory-server
install -m 755 target/release/surreal-memory-server ~/.local/bin/surreal-memory-server

# 3. Reload (bootout, bootstrap, enable, kickstart)
<skill-pack>/scripts/prometheus-services.sh load \
  --exclude surrealdb-native --exclude pk-cherry --exclude forge-mcp --exclude prometheus-nudge
```

`--exclude` takes the label without the `ai.prometheus.` prefix. If
`bootstrap` fails with `Input/output error` right after a `bootout`, the old
process is still shutting down; wait for it to exit and bootstrap again.

Verify:

```bash
curl -s 127.0.0.1:28000/version      # surrealdb-3.3.0
curl -s 127.0.0.1:23001/health       # status ok
curl -s 127.0.0.1:23001/ready        # every capability true
lsof -nP -iTCP:23001 -sTCP:LISTEN    # bound to 127.0.0.1 only
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
