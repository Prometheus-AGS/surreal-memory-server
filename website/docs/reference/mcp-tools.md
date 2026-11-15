---
id: mcp-tools
title: MCP Tools
sidebar_position: 1
---

# MCP Tools

The server exposes **59 tools**, grouped by concern. All are available over
stdio and HTTP.

## Connecting

| Transport | Endpoint |
|---|---|
| stdio | run the binary as the MCP server process |
| streamable HTTP | `POST /mcp/http` |
| legacy SSE | `GET /mcp/sse`, then `POST /mcp/messages?sessionId=…` |

Over HTTP, `initialize` returns an `mcp-session-id` header; send it on every later
request, and send `notifications/initialized` before the first tool call. In
`AUTH_MODE=jwt` each request also carries `Authorization: Bearer <token>`, and the
session belongs to the principal that opened it
([details](../guides/authentication.md#mcp-sessions)).

```bash
U=http://127.0.0.1:3001/mcp/http
H=(-H 'content-type: application/json' -H 'accept: application/json, text/event-stream')

# 1. initialize, and read the session id from the response headers
SID=$(curl -s -D - -o /dev/null -X POST $U "${H[@]}" -d '{"jsonrpc":"2.0","id":1,
  "method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},
  "clientInfo":{"name":"curl","version":"1"}}}' | awk -F': ' 'tolower($1)=="mcp-session-id"{print $2}' | tr -d '\r')

# 2. acknowledge, then call tools
curl -s -X POST $U "${H[@]}" -H "mcp-session-id: $SID" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'

curl -s -X POST $U "${H[@]}" -H "mcp-session-id: $SID" -d '{"jsonrpc":"2.0","id":2,
  "method":"tools/call","params":{"name":"add_memory","arguments":{
    "content":"Alice prefers dark mode","user_id":"alice","categories":["preference"]}}}'
```

Responses arrive as server-sent events (`data: {...}`); the tool's output is in
`result.content[0].text`.

## Scoped memory

`add_memory` · `get_memory` · `update_memory` · `delete_memory` ·
`delete_all_memories` · `get_all_memories` · `search_memories` ·
`hybrid_search_memories` · `get_memory_history` · `compress_memories` ·
`add_memories_from_conversation`

**Breaking change (issue #62):** `get_all_memories` returns one page,
`{ "memories": [...], "next_cursor": "memory:…" | null, "has_more": bool }`,
instead of a bare array of every memory. Pass `next_cursor` back as `cursor`
until it is `null`:

```json
{ "name": "get_all_memories",
  "arguments": { "user_id": "alice", "limit": 25 } }
{ "name": "get_all_memories",
  "arguments": { "user_id": "alice", "limit": 25,
                 "cursor": "memory:0b6c4d1e-1f2a-4c3b-9d8e-7f6a5b4c3d2e" } }
```

`limit` defaults to 50 and may be at most 200; with `include_embeddings: true`
the default and maximum are 25. Out-of-range values are rejected (`invalid
params`), never clamped. A cursor that is not of the form `memory:<key>` is
rejected the same way; any `memory:<key>` is accepted as a starting position,
and the scope filters still decide what is returned. A page also stops at 1 MiB of encoded
memories, so it can hold fewer than `limit` while `has_more` is `true`.
Embeddings are omitted unless `include_embeddings` is `true`. Pages are ordered
by record ID and are not a snapshot: memories added during iteration may be
missed, and deleted or expired ones do not reappear. String-encoded arguments
(`"limit": "25"`, `"include_embeddings": "false"`) are accepted.

`delete_all_memories` deletes in bounded batches of record IDs (expired and
archived memories included) and returns the count. It stops at the first memory
it cannot delete, and the error states how many were deleted before it. Calling
it again deletes the rest only once the cause is resolved; until then it stops at
the same memory.
`generate_persona_mindmap` reads the user's memories page by page.

`hybrid_search_memories` is the primary retrieval path — BM25 and HNSW merged by
reciprocal rank fusion. `search_memories` is vector-only and useful when you want
purely semantic matching.

```json
{ "name": "hybrid_search_memories",
  "arguments": { "query": "editor theme", "user_id": "alice",
                 "categories": ["preference"], "limit": 5 } }
```

Optional arguments: `agent_id`, `session_id`, `categories` (any-match),
`vector_weight` (default `0.7`), `bm25_weight` (default `0.3`) and
`include_embeddings` (default `false`).

`compress_memories` and `auto_summarize_task_stream` publish evidence-backed
summaries and archive their sources; see [Compression](../guides/compression.md):

```json
{ "name": "compress_memories",
  "arguments": { "user_id": "alice", "agent_id": "claude-code", "older_than_days": 30 } }
```

## Knowledge graph

`create_entity` · `create_entities` · `add_observations` · `create_relation` ·
`create_relations` · `read_graph` · `search_entities` · `semantic_search` ·
`delete_entity` · `delete_relation` · `get_relations` · `get_entity` ·
`update_entity` · `get_entity_history` · `get_graph_at_time`

Graph-RAG traversal: `find_path` · `expand_neighbors` · `get_related`

```json
{ "name": "create_entity",
  "arguments": { "name": "Alice", "entity_type": "person",
                 "observations": ["Prefers dark mode"] } }

{ "name": "create_relation",
  "arguments": { "from": "Alice", "to": "Acme", "relation_type": "works_at" } }
```

## TaskStreams

`create_task_stream` · `get_task_stream` · `add_to_task_stream` ·
`get_context_for_task` · `list_task_streams` · `archive_task_stream` ·
`auto_summarize_task_stream` · `pause_task_stream`

Steps: `add_task_step` · `update_task_step_status` · `get_task_steps` ·
`get_current_step` · `complete_step`

`get_context_for_task` sorts by importance and truncates to a token budget. It is
the hot path for prompt construction.

```json
{ "name": "auto_summarize_task_stream",
  "arguments": { "stream_name": "release-1-12", "agent_id": "claude-code",
                 "model_id": "gpt-4o" } }
```

## Mindmaps

`create_mindmap` · `get_mindmap` · `list_mindmaps` · `add_mindmap_node` ·
`add_mindmap_edge` · `delete_mindmap_node` · `delete_mindmap` ·
`export_mindmap` · `generate_persona_mindmap` · `generate_ideation_mindmap`

:::warning Size limit
Keep mindmaps under 500 nodes. They are stored as a single record with nested
arrays, so every node addition rewrites the whole document — a known SurrealDB
bottleneck with `UPDATE CONTENT` on large objects. Updates carry a 30s timeout
so an oversized write fails fast instead of stalling the write path.
:::

## Memory Palace (`palace` feature)

`palace_wake_up` · `palace_recall` · `palace_search` · `palace_ingest` ·
`palace_delete` · `palace_status` · `palace_hybrid_search`

Palace uses an independent 384-dimension space. `palace_hybrid_search` runs three
concurrent searches (memory, entity, drawers) and merges via RRF — the most
comprehensive and most expensive retrieval available.

## Argument coercion

Some MCP clients serialize every argument as a string (`"3"` for a number,
`"[\"a\"]"` for an array). Non-string parameters therefore use lenient
deserializers from `src/coerce.rs`. Without them the server rejects such calls
with `-32602 invalid type`.

These two calls are equivalent:

```json
{ "name": "hybrid_search_memories", "arguments": { "query": "theme", "limit": 3 } }
{ "name": "hybrid_search_memories", "arguments": { "query": "theme", "limit": "3" } }
```

## Scope and access

`user_id`, `agent_id` and `session_id` are **filters**, not access control. In the
default `local` mode they scope what a call sees. For a shared deployment, run with
`AUTH_MODE=jwt` so the tenant and caller come from a verified credential; then the
database a call reaches is chosen by the token, not by the arguments. See
[Authentication and tenancy](../guides/authentication.md).
