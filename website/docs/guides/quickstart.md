---
id: quickstart
title: Quickstart
sidebar_position: 1
---

# Quickstart

Run the server locally, store a memory, find it again, and connect an MCP client.
Every command on this page was run against a real server.

## 1. Start the server

For a single machine with an embedded database:

```bash
export SURREAL_MODE=embedded
export SURREAL_PATH="$HOME/.surreal-memory/db"
export EMBEDDING_PROVIDER=local          # BAAI/bge-small-en-v1.5, 384 dimensions
export API_PORT=3001

cargo build --release --features embedded,local-embeddings
./target/release/surreal-memory-server
```

With no other settings the server binds to `127.0.0.1` in `AUTH_MODE=local`, which
serves an unauthenticated API for the trusted local client. To run a database
server instead, use `SURREAL_MODE=server` and see
[Configuration](../reference/configuration.md).

Check it is up:

```bash
curl -s http://127.0.0.1:3001/health
# {"service":"surreal-memory-server","status":"ok","version":"1.11.0"}

curl -s http://127.0.0.1:3001/ready
# {"capabilities":{"coordinator":true,"ledger":true,"model_executor":true,
#   "search_index":true,"storage":true,"tokenizer":true},"status":"ready", ...}
```

`/ready` reports 503 until the database handle and embedding model are usable.

## 2. Store a memory

```bash
curl -s -X POST http://127.0.0.1:3001/api/v1/memory \
  -H 'content-type: application/json' \
  -d '{
        "content": "Alice prefers dark mode in the editor",
        "user_id": "alice",
        "agent_id": "claude-code",
        "categories": ["preference"]
      }'
```

The response is the stored memory, with `version: 1`, a `memory_type` of
`semantic`, and an `id` you can use for `GET`, `PUT`, `DELETE` and
`/history`:

```json
{
  "id": { "table": "memory", "key": { "String": "949ae93e-62b2-467a-8937-51739d240d9a" } },
  "content": "Alice prefers dark mode in the editor",
  "user_id": "alice",
  "agent_id": "claude-code",
  "categories": ["preference"],
  "memory_type": "semantic",
  "importance": 0.5,
  "version": 1
}
```

In a URL the id is `memory:<key>` or just `<key>`:

```bash
curl -s http://127.0.0.1:3001/api/v1/memory/memory:949ae93e-62b2-467a-8937-51739d240d9a
```

## 3. Search

Hybrid search merges BM25 and vector results by reciprocal rank fusion, so the
query does not need to share words with the memory:

```bash
curl -s -X POST http://127.0.0.1:3001/api/v1/search \
  -H 'content-type: application/json' \
  -d '{"query": "editor theme", "user_id": "alice", "limit": 3}'
```

Embedding vectors are left out of the response unless you pass
`"include_embeddings": true`.

## 4. Connect an MCP client

Over HTTP, point the client at the streamable endpoint:

```json
{
  "mcpServers": {
    "surreal-memory": { "type": "http", "url": "http://127.0.0.1:3001/mcp/http" }
  }
}
```

Over stdio, run the binary as the server process (the default when `MCP_STDIO` is
not `false`):

```json
{
  "mcpServers": {
    "surreal-memory": {
      "command": "/usr/local/bin/surreal-memory-server",
      "env": { "SURREAL_MODE": "embedded", "SURREAL_PATH": "/Users/me/.surreal-memory/db" }
    }
  }
}
```

The legacy SSE transport is at `/mcp/sse` and `/mcp/messages`. See
[MCP tools](../reference/mcp-tools.md) for the tools and example calls.

## Next

- [Authentication and tenancy](./authentication.md) before you expose the API
- [Durable operations](./durable-operations.md) for retry-safe writes
- [REST API](../reference/rest-api.md) for every route
