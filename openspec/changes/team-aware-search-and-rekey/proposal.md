# Team-aware search and re-key

## Why

The skill system's team-aware learning work keys every memory to one visibility level per `agent_id` and filters recall by `categories`. Three things in this server blocked that:

- **Search responses carried full embedding vectors.** They are many times larger than the text, and callers rank on the returned order.
- **The `categories` filter was advertised but dropped.** `search_memories` accepted it as `_categories` and ignored it, and `hybrid_search_memories` had no parameter for it at all.
- **Records written with `agent_id` NONE, NULL or `""` could not be repaired.** The pk learning worker wrote every record that way before prometheus-knowledge-rs#32.

This change records #44, which merged without an OpenSpec change.

## What Changes

- **Lean search responses.** `POST /api/v1/search`, the MCP `search_memories` tool and the MCP `hybrid_search_memories` tool omit `embedding` unless the request sets `include_embeddings: true`. Storage still loads embeddings for re-ranking.
- **`categories` (any-match) applied for real.**
  - It is now applied inside the KNN query and in the BM25 leg.
  - The storage trait's `hybrid_search_memories` gains a `categories` parameter, and every implementation and caller is updated.
  - REST `SearchBody` and the MCP hybrid tool accept it.
  - An empty list means no filter.
- **New v2 operation kind `rekey_agent_id`.**
  - Re-keys unattributed `memory` and `task_stream` records.
  - Supports `dry_run` and reuses the ledger's idempotency.
  - A task-stream unique-index collision is counted, not fatal.
  - **Loopback only:** the server serves with connect-info and returns 403 otherwise, failing closed when no connect-info is present.
  - The OpenAPI `OperationKind` enum is updated.
- **Version 1.10.0.** The OpenAPI document version is bumped too.

## Consumers

| Consumer | Effect |
|---|---|
| prometheus-skill-system (memory bridge, recall) | Intended consumer: uses `categories`, the lean responses and `rekey_agent_id`. |
| pk learning worker | Unchanged contract (`/ready`, `/api/v2/operations`). |
| Callers that read `embedding` from search results | Must send `include_embeddings: true`. No caller in the estate reads it. |
| Container deployments | `rekey_agent_id` is unavailable from bridge networks by design. Run it on the host. |

## Known issue recorded, not changed

An `add_memory` call without an `agent_id` de-duplicates (similarity > 0.92) across **all** agents in the user scope, so it can overwrite another agent's memory. Attributed writers always set `agent_id`. A follow-up should scope this de-duplication.
