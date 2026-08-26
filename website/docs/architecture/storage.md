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

## Bounded concurrency in embedded mode

RocksDB's `PointLockManager` defaults to 16 stripes per column family, so
concurrent transactions on overlapping keys serialize at the storage engine.
Layering an application lock on top compounds that into lock-timeout and
serialization-failure errors.

The fix is a semaphore sized to the stripe count
(`SURREAL_EMBEDDED_MAX_INFLIGHT`, default 16), which converts queueing into
honest backpressure. Server mode does not use it — the remote database schedules
its own work.

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

## Schema strictness

Tables are `SCHEMAFULL`. Consequences worth internalizing:

- A field on a Rust struct with no matching `DEFINE FIELD` fails at runtime with
  `Found field X, but no such field exists`.
- `option<object>` accepts `None` or a flat object but **rejects nested JSON**.
  Use `FLEXIBLE TYPE option<object>` for arbitrary structures.
- All DDL uses `IF NOT EXISTS`, so migrations are safe to re-run.
