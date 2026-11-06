---
id: storage
title: Storage Layer
sidebar_position: 2
---

# Storage Layer

`SurrealStorage` implements `MemoryStorage` over SurrealDB.

## Connection handling

The connection lives in an `Arc<ArcSwap<ConnectionCell>>`. Hot-path readers do a
single atomic load; reconnects do a single atomic store.

:::info `Surreal<Any>` is already `Arc`-wrapped
Do **not** wrap the client in `RwLock` or `Mutex`. It is clone-safe and the SDK
multiplexes internally. An earlier revision used `Arc<std::sync::RwLock<_>>`,
which serialized every storage call through a blocking lock and produced writer
starvation under load. The lock was the bug, not the protection.
:::

## The embedded engine

Embedded mode connects to `surrealkv://<SURREAL_PATH>`. SurrealDB 4.0 removed the
RocksDB engine; SurrealKV replaces it. A data directory written by SurrealDB 3.x on
RocksDB is converted in place on the first 4.0 start. That conversion is **one-way**,
so take an export or a copy of the directory first. One process owns a SurrealKV
directory at a time; use server mode when several processes need the data.

## Bounded concurrency in embedded mode

Concurrent transactions on overlapping keys serialize at the storage engine.
Layering an application lock on top compounds that into lock-timeout and
serialization-failure errors.

The fix is a semaphore (`SURREAL_EMBEDDED_MAX_INFLIGHT`, default 16), which converts
queueing into honest backpressure. Server mode does not use it — the remote database
schedules its own work. The default was sized on RocksDB's 16 lock stripes and has
not been re-measured on SurrealKV.

## Retry classification

Errors are classified into `Retry`, `Reconnect`, or `FailFast` rather than
retried uniformly. A schema rejection is not a transient fault, and retrying it
only delays the error.

Transaction conflicts are retried with bounded attempts and exponential backoff.
An unbounded retry loop turns a conflict that never clears into a hang while
spinning against the contention it is waiting on.

## Transactional writes

Multi-statement writes are wrapped in `BEGIN`/`COMMIT` and checked. `delete_memory`
is the clearest case: the audit row and the deletion commit together, so a reader
cannot observe the memory after deletion returned, and a failure between the two
cannot leave a `deleted` history row for a live memory.

## Ownership-qualified writes

A caller restricted to one user must not change another user's memory by guessing its
ID, and checking ownership with a read followed by an unqualified write would race
with a concurrent change. `update_memory_owned` and `delete_memory_owned` therefore
evaluate the owner predicate **inside the same transaction as the write**. A missing
memory and one owned by someone else are reported identically as "not found".

Both are methods of `MemoryStorage` with default bodies, so existing implementors
keep compiling; `SurrealStorage` overrides them with the in-transaction version. An
implementation that serves restricted callers must do the same.

## One database per tenant

In `AUTH_MODE=jwt` there is no single shared storage. A registry creates a
`SurrealStorage` for each tenant on first use, from the verified token's tenant: a
namespace `<SURREAL_NAMESPACE>_<tenant>` in server mode, or a directory
`<SURREAL_PATH>/tenants/<tenant>` in embedded mode. Each gets its own migrations,
operation ledger and cleanup worker. See
[Authentication and tenancy](../guides/authentication.md).

## Query embedding cache

`search_memories` keeps a bounded, per-storage cache of query embeddings. A key
contains the provider and model identity, the embedding dimensions, and the query
text normalized to NFC with whitespace collapsed, so two models can never share an
entry, and a provider that cannot name its identity bypasses the cache. Search
results and stored embeddings are never cached here.

Capacity is `SURREAL_MEMORY_QUERY_EMBED_CACHE` (default `512` entries). Hits, misses
and size are reported under `query_embed_cache` in `GET /api/v2/operations/stats`.

## Schema strictness

Tables are `SCHEMAFULL`. Consequences worth internalizing:

- A field on a Rust struct with no matching `DEFINE FIELD` fails at runtime with
  `Found field X, but no such field exists`.
- `option<object>` accepts `None` or a flat object but **rejects nested JSON**.
  Use `FLEXIBLE TYPE option<object>` for arbitrary structures.
- All DDL uses `IF NOT EXISTS`, so migrations are safe to re-run.
