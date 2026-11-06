---
id: memory-expiry
title: Memory expiry
sidebar_position: 4
---

# Memory expiry

A memory can carry an expiry time. Once it has passed, the memory is **invisible**:
it is excluded from every normal read straight away, whether or not the cleanup
worker has run. Cleanup only deletes it later.

## Setting an expiry

`valid_until` is a field of the library's `Memory` type. The REST and MCP request
bodies do not accept it, so today it is set by Rust code that uses the
`surreal-memory` crate (for example the Universal Agent Runtime). Responses always
include the field, as `null` when no expiry is set.

```rust
use chrono::{Duration, Utc};
use surreal_memory::{Memory, MemoryStorage};
use surrealdb::types::Datetime;

let mut memory = Memory::new(
    "Release freeze until Monday".to_string(),
    Some("alice".to_string()), // user_id
    None,                      // agent_id
    None,                      // session_id
    vec!["status".to_string()],
);
memory.valid_until = Some(Datetime::from(Utc::now() + Duration::days(3)));

let stored = storage.add_memory(memory).await?;
```

## What "expired" means

A memory is expired when `valid_until` is **strictly before** the cutoff. The cutoff
is read once at the start of each operation, so a single call sees one consistent
moment.

| `valid_until` vs. the cutoff | State |
|---|---|
| after | live |
| **equal** | live |
| one tick before | expired |

## Where it is enforced

An expired memory does not appear in:

- get by ID, and list
- vector search and both legs of hybrid search. The check happens **before** the
  result limit is applied, so expired rows can never take a slot from live ones
- TaskStream context
- compression and summarization inputs
- the near-duplicate check on a v1 `add_memory`

It also cannot be changed back to life:

- updating an expired memory is **refused**, not treated as a renewal
- a near-duplicate write never merges into it
- compression never copies its text into a live summary

## Cleanup

The TTL worker deletes expired memories every `TTL_INTERVAL_SECS` seconds (default
`3600`; a value that is not a positive integer is ignored with a warning). For each
memory it writes a `deleted` row to the memory history and removes the record.

The worker reports what it **actually removed**. A pass produces a `SweepReport`:

| Field | Meaning |
|---|---|
| `selected` | expired rows found this pass (one batch at most) |
| `removed` | rows confirmed deleted |
| `skipped` | rows that were renewed, or already gone, by the time they were deleted |
| `failed` | rows whose deletion failed |

A failed deletion is logged as an error, the row stays (still hidden) and the next
pass retries it:

```text
ERROR TTL sweep incomplete: failed deletions will be retried on the next pass
```

Library callers get the same information from
`MemoryStorage::expire_stale_memories_report`. `expire_stale_memories` returns an
error that wraps `SweepIncomplete { selected, removed, failed }` when anything failed,
which you can downcast.

In `AUTH_MODE=jwt`, each tenant has its own worker and its own expired rows.

## Auditing

Expiry is a visibility guarantee, **not erasure**. History rows are kept by design.
Audit and cleanup code can still reach an expired record:

```rust
// Returns the memory even if it has expired. This is a method of `SurrealStorage`
// itself, not of the `MemoryStorage` trait.
let record = storage.get_memory_including_expired(&id).await?;

// The full change history, including the final `deleted` row
let history = storage.get_memory_history(&id).await?;
```

Normal `get_memory` returns `None` for an expired memory.
