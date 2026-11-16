# Memory enumeration baseline (issue #62)

Measured 2026-11-15 on synthetic data only. SurrealDB SDK and server 4.0.0.
Embedded mode is the in-process SurrealKV store from `SurrealStorage::new_mem`;
server mode is a throwaway in-memory server
(`~/.prometheus/bin/surreal start --bind 127.0.0.1:28100 --user root --pass root memory`),
never the shared `:28000` instance.

Reproduce:

```bash
cargo test -p surreal-memory --test memory_enumeration -- --nocapture keyset_query_plan
TEST_SURREAL_ENDPOINT=ws://127.0.0.1:28100 cargo test -p surreal-memory \
  --test memory_enumeration -- --ignored --nocapture measure_ baseline_ server_keyset_query_plan
```

## 1. Query plan (task 1.3)

Query (as built by `enumeration.rs`, second page, 50 + 1 rows):

```sql
SELECT * OMIT embedding FROM memory
WHERE user_id = $u AND (status IS NONE OR status = 'active')
  AND (valid_until IS NONE OR valid_until >= $now) AND id > $after
ORDER BY id ASC LIMIT 51
```

Plans printed by `keyset_query_plan_uses_the_owner_index_without_sorting`
(embedded) and its `server_` twin. Both modes produced the same operator tree:

| Query | Plan (outermost first) |
|---|---|
| with `user_id` filter | `SelectProject (omit: embedding)` → `Limit 51` → `Filter (live view AND id > $after)` → `IndexScan memory_user (= 'plan', Forward)` |
| no owner filter | `SelectProject (omit: embedding)` → `Limit 51` → `Filter (live view AND id > $after)` → `TableScan memory (Forward)` |

Findings:

- **No sort.** `ORDER BY id ASC` is satisfied by scan order in both plans: a
  table scan walks record keys in order, and the `memory_user` index entries
  for one `user_id` are ordered by record ID. The `Limit` stops the scan at
  51 rows, so a page does not scan and sort the whole table.
- **Owner filter uses the existing index.** No composite index is needed, so
  **no migration was added.**
- **The cursor is a filter, not a range seek.** `id > $after` is evaluated as a
  `Filter` above the scan, so page *k* re-reads the index entries (or table
  rows) of the pages before it. Read from the plan (not timed), a full
  enumeration is quadratic in index reads for the scope (or the table, with no
  owner filter), though each database response stays bounded at `limit + 1`
  rows. Not addressed by this change.
- `OMIT embedding` appears as a projection attribute; the plan carries no
  embedding values. The same tree was seen with the 4.0.0 CLI against the
  scratch server before any code was written.

## 2. Page sizes (task 5.1)

`measure_page_sizes_embedded` / `measure_page_sizes_server`: 400 synthetic
memories of 32 KiB content each (13,107,200 content bytes, 12.5 times the
1 MiB budget), 1536-dimension embeddings. "Page bytes" is the JSON encoding of
the page's `memories` array; the budget is 1,048,576 bytes.

Re-measured after the size pre-pass (task 6.1; see section 4):

| Mode | Embeddings | Limit | Pages | Memories per page | Largest page (bytes) |
|---|---|---|---|---|---|
| embedded | no | 200 | 13 | 31 (last 28) | 1,030,167 |
| embedded | yes | 25 | 24 | 17 (last 9) | 890,066 |
| server | no | 200 | 13 | 31 (last 28) | 1,030,172 |
| server | yes | 25 | 24 | 17 (last 9) | 889,932 |

Every page was within the budget, and every memory was returned exactly once.
In both cases the byte budget, not the row limit, ended each page. With
embeddings the pre-pass estimate (16 bytes per vector value, 1 KiB per row for
other fields) is conservative, so pages hold 17 rather than the 20 that fit.

Before the pre-pass, the same runs gave 20 memories per page with embeddings
(largest 1,046,980 embedded / 1,046,970 server) and the same figures without.

## 3. Pre-change unbounded query (task 5.2)

`baseline_unbounded_query_server`, scratch server only. It seeds 200
synthetic memories of 400 KiB content each (81,920,000 content bytes, plus
1536-dimension embeddings) in one user scope, then runs the pre-change query
shape, `SELECT * FROM memory WHERE user_id = $u` (no `LIMIT`, `ORDER BY` or
projection), on a handle from `SurrealStorage::db()`.

Result (one run, debug build):

```
BASELINE unbounded: error after 234 ms: Connection reset
BASELINE paged after unbounded: memories=200
```

- **The failure reproduced** as `Connection reset`, the error reported in
  issue #62, 234 ms after the query was sent.
- The encoded size of the response was **not measured**: the response never
  arrived. All that is known is the size of what was stored (about 78 MiB of
  content plus vectors). This run does not establish the threshold; it is
  consistent with the 64 MiB SDK WebSocket message limit recorded in
  `docs/lessons.md` (2026-09-26), but no size between 3.75 MB (the working
  scoped call in the issue) and this one was tried.
- The reset's reason was not captured (the SDK logs it only at `trace`).
- Afterwards, the storage's paged listing over the same scope
  (`list_memories_page`, default limit 50, no embeddings) returned all 200
  memories.

## 4. Maximum limit over large memories (task 6.1)

Review found that the first implementation applied the byte budget only after
the database had sent `limit + 1` full rows: `limit=200` over 400 KiB memories
asked for about 80 MiB in one response, the failure in section 3, through a
legal request. Each page now runs two queries:

```sql
-- 1. sizes only, same filters, live view and cursor
SELECT id, bytes::len(<bytes>content) AS content_bytes,
       array::len(embedding ?? []) AS embedding_dims
FROM memory WHERE <owner filters> AND <live view> [AND id > $after]
ORDER BY id ASC LIMIT <limit + 1>;
-- 2. exactly the accepted range, existing projection
SELECT * [OMIT embedding] FROM memory
WHERE <owner filters> AND <live view> [AND id > $after] AND id <= $last
ORDER BY id ASC LIMIT <accepted>;
```

Rows are accepted in key order until the estimate (content bytes, plus 16 bytes
per vector value when embeddings are requested, plus 1 KiB per row) would pass
the budget; at least one is always accepted. Both expressions were checked on
the 4.0.0 CLI first: `bytes::len(<bytes>content)` counts UTF-8 bytes (`'abc€'`
is 6), and `array::len(embedding ?? [])` is 0 for a NONE embedding, without an
error. The assembled page is still trimmed by its real encoded size.

`server_the_maximum_limit_over_400_kib_memories_is_bounded_and_complete`
(scratch server, 200 memories × 400 KiB in one scope, `limit=200`):

```
LARGE count=200 bytes_each=409600 limit=200 pages=100 page_counts=[2, 2, …, 2]
```

Every memory was returned exactly once, there was no connection reset, and no
page asked the database for more than two memories' content. The embedded twin
(30 × 100 KiB, `limit=200`) gave 3 pages of 10.

## 5. Cursor round trip (task 6.2)

A cursor is rebuilt as a string record key. A memory created with a UUID-typed
key (`CREATE type::record('memory', rand::uuid::v4())`) does not round-trip: on
4.0 the rebuilt string key sorts before the UUID key, so the next page started
at the same memory again. This reproduced in both modes
(`a_cursor_that_does_not_advance_never_repeats_a_page` and its server twin), and
the page now fails with "memory page did not advance past its cursor" instead
of repeating forever in `get_all_memories` and the persona collector. Memories
the library writes have string keys, so this affects only rows created outside
it; such a scope cannot be enumerated past that row.
