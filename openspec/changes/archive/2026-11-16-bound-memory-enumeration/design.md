# Design

## Context

See `proposal.md` for motivation. What the code does today, read on `main` at
`0e9ffe6`:

- `SurrealStorage::list_memories_scoped`
  (`crates/surreal-memory/src/storage/surreal/expiry.rs`) builds
  `SELECT * FROM memory WHERE <owner filters> [AND live view]` with no
  `LIMIT`, `ORDER BY` or projection. `get_all_memories` calls it with the live
  view; `delete_all_memories` calls it with the physical view and then deletes
  row by row.
- MCP `get_all_memories` (`src/mcp/handlers.rs`) pretty-prints the full
  `Vec<Memory>`. REST `GET /api/v1/memory` (`src/api/memory.rs`) returns it as
  a JSON array. Both persona generators (`src/mcp/handlers.rs`,
  `src/api/mindmaps.rs`) load the full list, then use only the first category,
  the first five items per category and 80 characters of each.
- Memory record keys are UUID v4 strings, so record IDs are unique, immutable
  and totally ordered, but not time ordered.
- The live view (`LIVE_MEMORY_SQL`) needs `$now` bound once per operation.

Constraints that shape the approach:

- UAR implements `MemoryStorage` itself and forwards every method, so a new
  trait method needs a default body, and that default must fail closed
  (`docs/lessons.md`, 2026-11-05).
- Existing `MemoryStorage` signatures must not change.
- `Connection reset` from the SDK hides its reason at `trace`, which release
  builds compile out (`docs/lessons.md`, 2026-09-26). The server cannot
  recover that reason, so diagnostics have to describe the request instead.
- No persisted struct gains a field, so no migration is involved.

## Goals / Non-Goals

**Goals:**

- No code path issues an unbounded `memory` query.
- One page type and one set of limits shared by the library, MCP and REST.
- Enumeration that needs no schema change and no server-side cursor state.

**Non-Goals:**

- Proving why the unscoped call reset the connection. The measurement task
  records page sizes; it does not claim a transport threshold.
- Raising SDK transport limits. That would move the threshold, not remove it.
- Paginating other collections (entities, task streams, mindmaps, history).
- Snapshot-consistent enumeration.
- Changing who may list or delete memories. Authorization stays as it is.
- Rolling the change into UAR. That follows the consumer-sync process.

## Decisions

### 1. Keyset pagination on the record ID

Query shape: `SELECT <projection> FROM memory WHERE <filters> AND id > $after
ORDER BY id ASC LIMIT $n`, fetching `limit + 1` rows to learn whether more
remain without a second query.

The record ID is the only column that is unique, immutable and already
present on every row. That gives the stated consistency for free: a row that
exists for the whole iteration is seen exactly once; a row inserted behind the
cursor is missed and one inserted ahead is seen; a deleted row stops
appearing.

Alternatives considered:

- `ORDER BY created_at, id`. Chronological pages are nicer to read, but
  `created_at` is not unique and the cursor becomes a compound value. No
  caller of the listing depends on order today.
- `LIMIT`/`START` offsets. Rows shift under concurrent writes, which produces
  the duplicates and gaps the issue rules out, and cost grows with the offset.
- A server-side snapshot cursor. It needs session state on a connection the
  codebase deliberately shares.

The implementer must confirm with `EXPLAIN` on SurrealDB 4.0 that the query
does not scan and sort the whole table when owner filters are present, and
record the plan in the measurement notes. If it does, the fallback is a
composite index beginning with the owner field, added as a migration.

### 2. The cursor is the last returned record key

`next_cursor` is the key of the last memory on the page. It is parsed as a
`memory` record key before use and a value that does not parse is an invalid
request. It carries no scope: the owner filters are applied again on every
page, so a cursor taken from another scope only sets a starting position
inside the caller's own scope. Nothing is signed or stored.

### 3. Projection with `OMIT`, embeddings opt-in

The default projection is `SELECT * OMIT embedding`, so vectors never leave
the database. `DbMemory.embedding` is already `Option`, so decoding is
unchanged. With `include_embeddings` the projection is `SELECT *`.

An explicit field list was rejected: it must be kept in step with `DbMemory`
by hand, and a missed field would silently drop data.

`OMIT` is not used anywhere in the repository yet. The first task verifies it
against SurrealDB 4.0 in both modes before anything else is built on it.

### 4. Limits live in the library

| Constant | Value |
|---|---|
| Default page size | 50 |
| Maximum page size | 200 |
| Maximum page size with embeddings | 25 |
| Page byte budget (encoded JSON) | 1 MiB |

They are constants in the library, validated in the storage method, so MCP,
REST and Rust callers cannot disagree. An out-of-range limit is an error
whose message contains "invalid", which the existing REST error mapping
already turns into a 400. Clamping was rejected because a caller that asked
for 1000 and received 200 has no signal that the request was changed.

They are not environment variables. Nothing yet shows an operator needs to
tune them.

### 5. Byte budget is applied before content leaves the database

A page is read in two queries. The first selects, for up to `limit + 1` rows
in key order, only the record ID and the stored sizes (content length, and
embedding length when embeddings are requested). Rows are accepted until the
estimated encoded size would exceed the budget, always accepting at least one.
The second query fetches exactly the accepted ID range with the same filters.

A single query with `LIMIT limit + 1` was the first implementation and review
found it insufficient: 200 rows of 400 KiB is about 80 MiB in one response,
which is the failure this change exists to remove.

The assembled page is still measured as serialized JSON and trimmed, because
the estimate ignores metadata and a row can grow between the two queries. A
page always keeps at least one memory. A page whose next cursor equals the
cursor it was requested with is an error, so a record key that does not
round-trip cannot repeat a page forever.

### 6. One new trait method, legacy method kept complete

`MemoryStorage` gains `list_memories_page(request) -> Result<MemoryPage>`
with a default body that returns an error. `MemoryPageRequest` holds the three
owner filters, `limit`, `cursor` and `include_embeddings`; `MemoryPage` holds
`memories`, `next_cursor` and `has_more`. Both are re-exported from `lib.rs`.

`get_all_memories` keeps its signature and its complete result, embeddings
included, because Rust callers may rely on both. `SurrealStorage` implements
it by looping over internal pages at the embedding page size and returns an
error if any page fails. Its memory use stays proportional to the scope; that
is the legacy contract, and its doc comment says so and points at
`list_memories_page`. It is not marked `#[deprecated]`: the attribute would
turn every existing in-repo test call into a `-D warnings` failure.

### 7. Transports return the page object

MCP `get_all_memories` and REST `GET /api/v1/memory` both serialize
`MemoryPage`. The MCP tool keeps its name. New MCP parameters use the lenient
deserializers in `src/coerce.rs` (`opt_number`, `opt_bool`) as the project
requires. REST keeps deriving the owner from the verified principal exactly as
it does now.

A separate `list_memories` tool was rejected: it leaves `get_all_memories` in
place with no honest thing to return.

### 8. Bulk delete pages IDs

`delete_all_memories` selects `id` only, physical view, ordered by ID, in
batches, and advances an ID cursor so a row that cannot be deleted is not
selected again in the same run. Each row is deleted through the existing
`delete_memory`, which reads that one row to write its history, so history
and derivation handling are unchanged. A row that cannot be deleted stops the
run; reruns stop at it again until it is repaired. On failure
the error is wrapped with the count deleted so far. The return type stays
`Result<u64>`.

The failure path is tested with the existing fault-injection hook
(`FaultPoint`), adding one point for bulk deletion.

### 9. Persona generation shares one paged collector

Both persona generators loop over pages without embeddings and keep, per
category, the first five items truncated to 80 characters plus a running
total. The loop is one shared function used by the MCP handler and the REST
handler, replacing two copies of the clustering code. Mindmap construction
stays where it is.

### 10. Diagnostics describe the request

The storage method attaches context to any query error: operation name, page
size, embedding flag, and three booleans for which owner filters were set. It
logs the same at `warn`. Filter values, content and credentials are never
included.

## Risks / Trade-offs

- [Two breaking transport responses] → Called out in the proposal, the docs
  and the PR. Clients fail on shape rather than reading a truncated list as
  complete.
- [One row can still be large] → The size pre-pass bounds a page's database
  response to the budget plus one memory, but a single memory's content has no
  size cap in storage. Capping content size is a separate change.
- [Each page re-reads the index entries before its cursor] → The cursor is a
  filter, not a range seek, so a full enumeration of a very large scope costs
  quadratic index reads. Responses stay bounded; late pages get slower. A
  record-range query for the unfiltered case is a follow-up.
- [Keyset query may not use an index with owner filters] → Checked with
  `EXPLAIN` in the first task group; fallback is a composite index migration.
- [`get_all_memories` still builds the whole set in memory] → Accepted to keep
  the library contract. Each database response is bounded, which is what
  resets the shared connection.
- [Not a snapshot] → Stated in the spec and the docs. A caller that needs a
  frozen view has to stop writes itself.
- [Root cause of the reset stays unproven] → The measurement notes report what
  was measured and say plainly whether the unbounded failure was reproduced.

## Migration Plan

1. Ship the library change. Rust callers are unaffected.
2. Ship the transport change in the same release, with the breaking response
   shape in the release notes and the reference docs.
3. Resync UAR through the consumer-sync process. Its `MemoryStorage`
   implementation keeps compiling; it gets the fail-closed default until it
   forwards `list_memories_page`.

Rollback is a revert of the release. There is no schema or data change to
undo.

## Open Questions

- Should memory content get a size cap at write time? It would close the
  remaining single-row risk and does not affect this change.
