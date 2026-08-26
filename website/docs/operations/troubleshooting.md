---
id: troubleshooting
title: Troubleshooting
sidebar_position: 2
---

# Troubleshooting

Symptoms observed in production, with the diagnosis that actually explained them.

## `model_executor: false`, search unavailable

The model has not loaded. Check the log for the resolved cache path:

```
INFO Resolving model from Hugging Face: BAAI/bge-small-en-v1.5 (cache: .../huggingface/hub)
```

If that path does **not** end in `hub`, the cache is misconfigured and the server
is re-downloading weights it may already have. `MODEL_CACHE_DIR` should name the
HuggingFace home; the server appends `hub`.

## `executor generation N was nonresponsive and restarted`

The supervisor's watchdog fired. Distinguish two cases:

**During startup** — if no `Loading Candle embeddings model` line preceded it,
the child died before it could read a request. Raise
`SURREAL_EXECUTOR_STARTUP_MS`. GPU initialization is highly variable and has been
measured from 1.2s to over 150s on the same host depending on load.

**While serving** — if the child had accepted a request, something genuinely
stalled. That is a real fault worth investigating.

## Everything is slow, including `/health`

`/health` is a static handler. If it takes tens of seconds, the process is not
being scheduled — check `uptime` before suspecting code.

Timing symptoms on a saturated machine are not evidence about the server. A load
average many times the core count will make every timeout look like a hang.

## `Found field X, but no such field exists`

A Rust struct has a field with no matching `DEFINE FIELD`. Add a migration; see
[Migrations](../reference/migrations.md).

## Nested JSON rejected on a metadata field

`option<object>` accepts `None` or a flat object but rejects nesting. The field
needs `FLEXIBLE TYPE option<object>`.

## Mindmap updates time out

Mindmaps over ~500 nodes hit a SurrealDB bottleneck with `UPDATE CONTENT` on
large nested objects. Updates carry a 30s timeout so they fail fast. Split large
hierarchies, or use TaskStreams for linear context.

## Empty search results for an agent

Check the scope. For CLI and agent consumers, `user_id` must be `"anonymous"` so
filtering happens on `agent_id`. Setting `user_id = agent_id` filters on a value
nothing was written under, and returns nothing.

## Embedded mode fails to open the database

RocksDB cannot be opened by two processes at once. Use server mode for
multi-process access.
