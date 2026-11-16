# Proposal

## Why

Listing memories is unbounded at every layer (issue #62). The storage query is
`SELECT * FROM memory WHERE ...` with no limit, cursor or projection, and the
MCP and REST transports serialize the whole result, embeddings included. On
2026-11-15 a scoped call returned 331 records in 3.75 MB, and the unscoped MCP
`get_all_memories {}` call repeatedly failed with `Connection reset`, which
took the shared database connection down for every other caller. Bulk delete
and persona mindmap generation load the same full collection first.

The cause of the reset is not proven. An oversized WebSocket response is the
leading hypothesis, and this change does not claim it was measured. The fix
does not depend on the exact threshold: no request should be able to ask the
database for an unbounded response.

## What Changes

- Add keyset-paginated memory listing to the storage library: a page request
  with a hard maximum size, a stable order, an opaque resumable cursor, and
  documented behavior under concurrent writes.
- Omit embeddings from listed memories by default, at query time. Embeddings
  are opt-in per request and carry a smaller page-size maximum.
- Bound each returned page by an encoded-byte budget as well as a row count.
- **BREAKING** MCP `get_all_memories` returns one page as an object
  (`memories`, `next_cursor`, `has_more`) instead of a bare array of every
  memory, and accepts `limit`, `cursor` and `include_embeddings`.
- **BREAKING** REST `GET /api/v1/memory` returns the same page object instead
  of a bare array, and accepts the same three query parameters.
- Keep the library's `MemoryStorage::get_all_memories` complete and source
  compatible for existing callers, but fetch it through bounded pages so no
  single database response is unbounded. Document it as legacy.
- Bulk delete pages record IDs only, never content or vectors, and reports how
  many memories it deleted before a failure so the caller can rerun it.
- Persona mindmap generation (MCP and REST) consumes pages and keeps only the
  bounded per-category state it renders.
- A failed listing reports and logs the operation, page size, embedding flag
  and which scope filters were present, never memory content, scope values or
  credentials.
- Record measured page sizes on synthetic data for embedded and server mode.

The two transport changes are breaking on purpose. A legacy client that
ignores a pagination header would read a truncated array as the complete set;
a changed body shape fails loudly instead.

## Capabilities

### New Capabilities

- `memory-enumeration`: listing, bulk deletion and whole-scope consumption of
  memories through bounded, resumable pages, including what a page contains,
  its size limits, its consistency under concurrent writes, and how failures
  are reported.

### Modified Capabilities

None. No existing capability specifies memory listing.

## Impact

- **Library** (`crates/surreal-memory`): new page request and page types, one
  new `MemoryStorage` method with a fail-closed default body (UAR implements
  the trait itself, so a required method would break it), and reimplemented
  `get_all_memories` and `delete_all_memories` in `SurrealStorage`. No
  persisted field is added, so no migration is needed.
- **MCP** (`src/mcp/`): `get_all_memories` parameters and response shape,
  persona mindmap generation, tool descriptions and the `instructions` text.
- **REST** (`src/api/memory.rs`, `src/api/mindmaps.rs`): list response shape
  and query parameters, persona mindmap generation.
- **Consumers**: MCP and REST clients that parse the list response as an array
  must read `memories` and follow `next_cursor`. Rust consumers of
  `get_all_memories` keep compiling and keep receiving every memory.
- **Docs**: `website/docs/reference/rest-api.md`,
  `website/docs/reference/mcp-tools.md`, `docs/MCP_REST_CONTRACT_MATRIX.md`.
- **Dependencies**: none added.
