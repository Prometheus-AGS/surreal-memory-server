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

An embedded SurrealKV directory cannot be opened by two processes at once. Use
server mode for multi-process access.

## The server refuses to start

`refusing to start: AUTH_MODE=local serves an unauthenticated API, but API_HOST=… is
not a loopback address` means you bound the API to a network address without
credentials. Either bind `API_HOST=127.0.0.1`, or set `AUTH_MODE=jwt` with
`AUTH_JWT_ISSUER`, `AUTH_JWT_AUDIENCE`, `AUTH_JWT_ALGORITHM` and a key. The message
for a missing setting names it. An HS256 key must be at least 32 bytes.

## Requests return 401, 403 or 503 in `jwt` mode

| Symptom | Likely cause |
|---|---|
| `401` on every request | no `Authorization: Bearer` header; wrong `iss` or `aud`; signed with another key; `exp` passed |
| `401` only for old tokens | `iat` is older than `AUTH_JWT_MAX_AGE_SECS` (default 300). Re-issue the token |
| `401` right after minting | the clock on the issuer is ahead; a token issued more than a few seconds in the future is refused |
| `403` "outside the credential's scope" | the token has `user_id` and the request named another user (or an empty one) |
| `403` "restricted to the memory and search routes" | a `user_id` token used another route |
| `403` "needs the operator permission" | retry, reject or re-key without `"ops":["operator"]` |
| `503` "tenant storage unavailable" | the tenant database could not be opened or created. In server mode the SurrealDB user needs the right to create namespaces; check the server log for the tenant name |

## A memory exists but cannot be found

Four things make a memory invisible without deleting it:

- it has **expired** (`valid_until` is in the past); see [Memory expiry](../guides/memory-expiry.md)
- it was **summarized**: its sources are archived and the summary is returned instead;
  see [Compression](../guides/compression.md)
- it belongs to **another tenant**, or to another user when your token has `user_id`
- the search scope names a different `user_id`, `agent_id` or `session_id`

## Cleanup reports failures

`TTL sweep incomplete: failed deletions will be retried on the next pass` is not
fatal. The rows stay hidden from every read and are retried each pass. If it repeats,
the log line before it names the cause.
