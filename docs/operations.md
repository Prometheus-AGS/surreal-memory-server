# Operations reference: team-scoping additions

The operations ledger lives at `/api/v2/operations` (`src/operations.rs`).
This page covers the additions made for team-aware learning (prometheus-skill-system
`docs/design/team-aware-learning-memory.md` §2, §8).

## Search responses are lean by default

`POST /api/v1/search` and the MCP `search_memories` and `hybrid_search_memories`
tools no longer return `embedding` vectors. A vector is many times larger than the
memory text, and callers rank on the returned order. Send `"include_embeddings": true`
to get the vectors back. Storage still loads the embeddings, because the server
re-ranks with them.

## Category filtering

The following accept `categories` (any-match):
- `POST /api/v1/search`;
- the MCP `hybrid_search_memories` tool;
- the MCP `search_memories` tool, which previously accepted the field and ignored it.

The filter applies inside the nearest-neighbour query and in the BM25 leg, so a
filtered search still returns up to `limit` matching rows even when none of them are
among the unfiltered top results. An empty list means "no filter".

## `rekey_agent_id` operation

`rekey_agent_id` re-keys **unattributed** records to an explicit `agent_id`. A record
is unattributed when its `agent_id` is absent (NONE), NULL, or `""`; task streams
store an absent id as `""` since migration v18.

```json
{
  "operation_id": "rekey-2026-10-04",
  "schema_version": 2,
  "kind": "rekey_agent_id",
  "dependencies": [],
  "payload_hash": "<sha256 of the payload JSON>",
  "payload": { "to_agent_id": "@project", "user_id": "project:…", "dry_run": true }
}
```

- **Scope:** `memory` and `task_stream` records. Task steps have no `agent_id`.
- **`dry_run: true`:** writes nothing. The committed receipt's `result` reports the
  counts (`memories`, `task_streams`).
- **Collisions:** a task stream whose `(agent_id, user_id, name)` already exists under
  the target id is left unchanged and counted in `task_stream_conflicts`.
- **Idempotency:** replaying the same `operation_id` returns the existing receipt
  (HTTP 200) and does not run again.
- **Loopback only:** the server refuses the operation with **403** unless the peer
  address is loopback. A server without connect-info fails closed. Container
  deployments reach the server from a bridge address, so the operation is
  unavailable there; run it on the host against the loopback listener.
