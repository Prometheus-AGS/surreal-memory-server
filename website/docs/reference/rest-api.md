---
id: rest-api
title: REST API
sidebar_position: 2
---

# REST API

The server serves REST, A2A and MCP on one port (`API_PORT`, default `3001`). Bodies
and responses are JSON, and errors are always `{"error": "<message>"}`.

**Authentication.** In the default `AUTH_MODE=local` no credential is needed (the
server only listens on loopback). In `AUTH_MODE=jwt` every route except `/health`
and `/ready` needs `Authorization: Bearer <token>`. The examples below leave the
header out; add it for a shared deployment. See
[Authentication and tenancy](../guides/authentication.md).

| Status | Meaning |
|---|---|
| `200`, `201`, `204` | success |
| `400` | invalid input |
| `401` | missing or invalid credential (`jwt` mode) |
| `403` | credential is valid but outside its scope |
| `404` | not found, **or** owned by someone your credential cannot see |
| `409` | conflict (for example an operation ID reused with another payload) |
| `500` / `503` | server or storage failure / temporarily unavailable |

## Health

| Method and path | Purpose |
|---|---|
| `GET /health` | liveness; static |
| `GET /ready` | readiness: `storage`, `ledger`, `coordinator`, `tokenizer`, `model_executor`, `search_index`; `503` when the database handle is unavailable |

## Memory

Base path `/api/v1/memory`.

| Method and path | Purpose |
|---|---|
| `POST /` | add a memory |
| `GET /` | list; filter with `?user_id=&agent_id=&session_id=` |
| `DELETE /` | delete every memory matching the same filters |
| `GET /{id}` | fetch one |
| `PUT /{id}` | replace its content (re-embeds, bumps `version`) |
| `DELETE /{id}` | delete one |
| `GET /{id}/history` | the change history |

```bash
# Add
curl -s -X POST http://127.0.0.1:3001/api/v1/memory -H 'content-type: application/json' \
  -d '{"content":"Alice prefers dark mode","user_id":"alice","agent_id":"claude-code",
       "session_id":"s-42","categories":["preference"]}'

# List one user's memories
curl -s 'http://127.0.0.1:3001/api/v1/memory?user_id=alice'

# Update, then read the history
curl -s -X PUT http://127.0.0.1:3001/api/v1/memory/memory:949ae93e-62b2-467a-8937-51739d240d9a \
  -H 'content-type: application/json' -d '{"content":"Alice prefers dark mode and large fonts"}'
curl -s http://127.0.0.1:3001/api/v1/memory/memory:949ae93e-62b2-467a-8937-51739d240d9a/history
```

`POST` requires `content`. Everything else is optional: `user_id`, `agent_id`,
`session_id`, `categories`. Adding text that is a near-duplicate (cosine similarity
at or above 0.92) of a memory in the same scope **updates that memory** instead of
creating another. A missing scope field widens the check to every value of it, so
set `user_id` and `agent_id` when you want the check to stay narrow.

`DELETE /` with no filters deletes every memory in the tenant. Always pass a scope.

## Search

`POST /api/v1/search` runs hybrid search (BM25 and HNSW merged by reciprocal rank
fusion).

| Field | Default | Meaning |
|---|---|---|
| `query` | required | the search text |
| `user_id`, `agent_id`, `session_id` | none | scope filters |
| `categories` | none | any-match filter, applied to both legs |
| `limit` | `10` | maximum results |
| `vector_weight` / `bm25_weight` | `0.7` / `0.3` | the blend |
| `include_embeddings` | `false` | embeddings are large and omitted unless asked for |

```bash
curl -s -X POST http://127.0.0.1:3001/api/v1/search -H 'content-type: application/json' \
  -d '{"query":"editor theme","user_id":"alice","categories":["preference"],"limit":5}'
```

## Knowledge graph

Base path `/api/v1/entities`.

| Method and path | Purpose |
|---|---|
| `POST /` · `POST /batch` | create entities |
| `GET /` | read the whole graph |
| `GET /search?q=…` | search entities |
| `DELETE /{name}` | delete an entity |
| `POST /{name}/observations` | add observations |
| `GET /{name}/neighbors?depth=&limit=` | neighbors up to a depth |
| `GET /{name}/related?relation_type=&direction=&limit=` | related entities |
| `GET /{name}/path/{to}?max_depth=` | shortest path |
| `POST /relations` · `POST /relations/batch` | create relations |

```bash
curl -s -X POST http://127.0.0.1:3001/api/v1/entities -H 'content-type: application/json' \
  -d '{"name":"Alice","entity_type":"person","observations":["Prefers dark mode"]}'

curl -s -X POST http://127.0.0.1:3001/api/v1/entities/relations -H 'content-type: application/json' \
  -d '{"from":"Alice","to":"Acme","relation_type":"works_at"}'
```

## TaskStreams

Base path `/api/v1/taskstreams`.

| Method and path | Purpose |
|---|---|
| `POST /` · `GET /` | create · list |
| `GET /{name}` · `DELETE /{name}` | fetch · delete |
| `POST /{name}/archive` · `POST /{name}/pause` | lifecycle |
| `POST /{name}/memories` | add a memory to the stream |
| `GET /{name}/context` | token-budgeted context window |
| `POST /{name}/summarize` | [compress](../guides/compression.md) the stream |

```bash
curl -s -X POST http://127.0.0.1:3001/api/v1/taskstreams -H 'content-type: application/json' \
  -d '{"name":"release-1-12","description":"Cut the 1.12 release","agent_id":"claude-code",
       "model_id":"gpt-4o","auto_summarize":true}'
```

## Mindmaps

Base path `/api/v1/mindmaps`.

| Method and path | Purpose |
|---|---|
| `POST /` · `GET /` | create · list |
| `GET /{name}` · `DELETE /{name}` | fetch · delete |
| `POST /{name}/nodes` · `DELETE /{name}/nodes/{node_id}` | nodes |
| `POST /{name}/edges` | add an edge |
| `GET /{name}/export` | export |
| `POST /generate/persona` · `POST /generate/ideation` | generate |

Keep mindmaps under 500 nodes; see [MCP tools](./mcp-tools.md#mindmaps).

## Durable operations

Base path `/api/v2/operations`. See [Durable operations](../guides/durable-operations.md).

| Method and path | Purpose |
|---|---|
| `POST /` | submit an operation |
| `GET /{id}` | the authoritative receipt |
| `GET /{id}/events?after=N` | ordered state events (server-sent events) |
| `GET /stats` | counts per state, oldest unfinished operation, query cache |
| `POST /{id}/retry` | re-drive now (**operator**) |
| `POST /{id}/reject` | dead-letter, body `{"reason":"…"}` (**operator**) |

## Memory Palace (`palace` feature)

Base path `/api/v1/palace`: `GET /wake`, `GET /recall`, `POST /search`,
`POST /ingest`, `DELETE /drawer/{id}`, `GET /status`, `POST /hybrid-search`.

## A2A

| Method and path | Purpose |
|---|---|
| `GET /agent.json` | the agent card; advertises `bearer` authentication |
| `POST /a2a/tasks/send` | create a task: `{"skill_id": "…", "input": {…}}` |
| `GET /a2a/tasks/{id}` | task status |

## MCP over HTTP

| Path | Transport |
|---|---|
| `/mcp/http` | streamable HTTP (current standard) |
| `/mcp/sse` and `POST /mcp/messages?sessionId=…` | legacy SSE |

Sessions are bound to the principal that opened them. See
[MCP tools](./mcp-tools.md) for the tools and example calls.
