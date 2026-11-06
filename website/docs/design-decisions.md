---
id: design-decisions
title: Design Decisions
sidebar_position: 5
---

# Design Decisions

The reasoning behind choices that are not obvious from the code, including the
ones that were originally wrong.

## Why a separate embedding process

Local embedding loads Candle plus a GPU backend — a stack that can hang in FFI
or abort in ways safe Rust cannot contain. In-process, any of those takes down
the server holding an agent's memory.

The cost is a JSON stdio protocol and a supervisor. The benefit is that a bad
model load kills a replaceable child. For a component every other service
depends on, that trade is worth making.

## Why startup and per-request budgets are separate

Originally one 30s watchdog covered both. That was wrong, and the failure was
instructive.

Before signalling readiness, the child is **structurally incapable of producing
output** — its heartbeat does not exist until it reads a request. A watchdog
covering that phase measures nothing and kills healthy processes. Observed GPU
init on one host: 1.2s, 14s, 36s, and over 150s across boots, tracking system
load. The 30s watchdog killed the child, the caller retried once, and the result
was a ~63s failure with no diagnostic — the child had never been able to log.

The general rule: **a watchdog must not cover a phase in which the watched
process cannot report progress.**

## Why `ArcSwap` and not a lock

An earlier revision held the SurrealDB client in `Arc<std::sync::RwLock<_>>`.
`Surreal<Any>` is already `Arc`-wrapped and clone-safe, so the lock added no
safety and serialized every call, producing writer starvation under load.

The lock was the bug. `ArcSwap` gives lock-free reads on the hot path and atomic
replacement on reconnect.

## Why a semaphore in embedded mode only

RocksDB serializes conflicting transactions across 16 default stripes. Exceeding
that produces lock-timeout errors that look like application faults. Bounding
in-flight operations to the stripe count converts the failure into honest
backpressure.

Server mode gets no semaphore — the remote database schedules its own work, and
adding a client-side bound would just be a guess about someone else's scheduler.

The number 16 comes from RocksDB's stripe count. SurrealDB 4.0 replaced RocksDB
with SurrealKV, which syncs every commit by default, and the bound has not been
re-measured on it. It is kept because it is a safe ceiling, not because it is a
proven optimum.

## Why one database per tenant

For shared deployments the isolation boundary is a database, chosen only from the
verified token. The alternative, one shared table with a `tenant_id` column, makes
isolation depend on every query remembering a predicate, and a single forgotten
`WHERE` becomes a cross-tenant leak. With a database per tenant a forgotten filter
can only ever see the caller's own data, and identifiers such as operation IDs
cannot collide across tenants. The cost is operational: each tenant is a namespace
to create, back up and migrate.

## Why authentication is default-deny

One middleware authenticates every request and exempts only `/health` and
`/ready`. A route added next year is therefore protected without anyone having to
remember to list it, and a test scans the source for routes so an unclassified one
fails the build.

## Why SCHEMAFULL despite the maintenance cost

Every persisted field needs a migration, which is real friction. The alternative
is silent data drift: a typo'd field name writes successfully and reads back as
absent, and the bug surfaces later as missing data with no error.

SCHEMAFULL turns that into an immediate, loud failure.

## Why reciprocal rank fusion

BM25 and HNSW produce incomparable scores; there is no principled threshold that
works for both. RRF combines **rank positions** instead, which needs no
calibration and degrades gracefully when one index returns nothing useful.

## Why the `MemoryStorage` trait is treated as published API

The Universal Agent Runtime depends on it and wraps every method with scope
enforcement. Changing the trait ripples into a consumer that cannot be updated
atomically, so it is versioned as an interface rather than refactored freely.

## Two vector spaces, deliberately not unified

Memory and entity embeddings are 1536-dimension; Palace drawers are 384. Unifying
them would require re-embedding every record, and the spaces serve different
retrieval characteristics. They are kept independent and must never be
cross-queried.
