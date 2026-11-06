---
id: whats-new
title: What's new
sidebar_position: 2
---

# What's new

Changes since the 1.10 line, newest first. Each entry links to the page that
documents it in full.

## Verified caller scope and tenant isolation

The REST, A2A and MCP HTTP surface can now run behind verified credentials
instead of trusting the network.

- **`AUTH_MODE=jwt`**: every route except `/health` and `/ready` requires a
  signed bearer token. A new route is protected by default.
- **One database per tenant.** The tenant comes only from the verified token and
  selects the SurrealDB namespace (server mode) or directory (embedded mode).
- **Safe default.** `API_HOST` now defaults to `127.0.0.1`, and
  `AUTH_MODE=local` is refused on any other bind address.
- **Operator permission.** Retry, reject and re-key of durable operations need
  an explicit `operator` claim.
- **User-restricted tokens** limit a credential to one user's memories.
- **MCP sessions** (streamable HTTP and legacy SSE) are bound to the principal
  that opened them.

See [Authentication and tenancy](guides/authentication.md).

:::warning Upgrade note
If you bound the server to a non-loopback address before, it will now refuse to
start until you either set `AUTH_MODE=jwt` with the `AUTH_JWT_*` settings or bind
`API_HOST` to `127.0.0.1`. The Docker Compose files now require the
`AUTH_JWT_*` variables and publish only on the host loopback.
:::

## Expired memory is invisible, and cleanup reports honestly

A memory with `valid_until` in the past no longer appears in any normal read, even
before the cleanup worker runs, and cannot be updated, renewed, merged into by a
near-duplicate write, or copied into a summary. The cleanup worker reports what it
actually removed, and a failed deletion is counted, logged and retried.
See [Memory expiry](guides/memory-expiry.md).

## Evidence-backed compression

`compress_memories` and `auto_summarize_task_stream` now publish summaries whose
claims quote their sources. A deterministic gate checks every quote and the token
budget; the sources are archived, not deleted, and stay readable as evidence.
The default summarizer is extractive and never leaves the process.
See [Compression](guides/compression.md).

## Hybrid search works as documented

The BM25 leg of hybrid search used a full-text index that did not exist and
swallowed the error, so hybrid search was silently vector-only. Migration v24
creates the index. See [Migrations](reference/migrations.md).

## SurrealDB 4.0 and SurrealKV

The server targets SurrealDB **4.0.0**. Embedded mode now runs on **SurrealKV**,
because 4.0 removed RocksDB. A RocksDB data directory is converted in place on the
first 4.0 start, which is one-way, so back it up first.
See [Storage](architecture/storage.md) and [Deployment](operations/deployment.md).

## Durable operations: retry, reject and stats

`POST /api/v2/operations/{id}/retry`, `POST /api/v2/operations/{id}/reject` and
`GET /api/v2/operations/stats` give operators control over stuck work, and
operations that can never succeed are rejected instead of being retried forever.
See [Durable operations](guides/durable-operations.md).

## Claude Code hooks and statusline

`surreal-memory-server hook session-start|pre-compact|session-end` and
`surreal-memory-server statusline` recall and persist session memory through the
running server. They never block the harness. See [CLI and hooks](reference/cli.md).

## Query embedding cache

Search keeps a bounded cache of query embeddings, keyed by the provider and model
identity so two models can never share an entry. Its hit and miss counts are in
`GET /api/v2/operations/stats`. Capacity is set with
`SURREAL_MEMORY_QUERY_EMBED_CACHE`. See [Configuration](reference/configuration.md).

## Migrations v22 to v24

| Version | Name | Purpose |
|---|---|---|
| 22 | `compression_derivations_and_claims` | Summary derivations and their claims |
| 23 | `compression_evidence_and_memory_status` | Evidence rows and archived-source status |
| 24 | `memory_fulltext_index` | The BM25 index hybrid search relies on |

All three are additive and apply automatically on startup.
