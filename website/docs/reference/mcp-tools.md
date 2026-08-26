---
id: mcp-tools
title: MCP Tools
sidebar_position: 1
---

# MCP Tools

The server exposes **59 tools**, grouped by concern. All are available over
stdio and HTTP.

## Scoped memory

`add_memory` · `get_memory` · `update_memory` · `delete_memory` ·
`delete_all_memories` · `get_all_memories` · `search_memories` ·
`hybrid_search_memories` · `get_memory_history` · `compress_memories` ·
`add_memories_from_conversation`

`hybrid_search_memories` is the primary retrieval path — BM25 and HNSW merged by
reciprocal rank fusion. `search_memories` is vector-only and useful when you want
purely semantic matching.

## Knowledge graph

`create_entity` · `create_entities` · `add_observations` · `create_relation` ·
`create_relations` · `read_graph` · `search_entities` · `semantic_search` ·
`delete_entity` · `delete_relation` · `get_relations` · `get_entity` ·
`update_entity` · `get_entity_history` · `get_graph_at_time`

Graph-RAG traversal: `find_path` · `expand_neighbors` · `get_related`

## TaskStreams

`create_task_stream` · `get_task_stream` · `add_to_task_stream` ·
`get_context_for_task` · `list_task_streams` · `archive_task_stream` ·
`auto_summarize_task_stream` · `pause_task_stream`

Steps: `add_task_step` · `update_task_step_status` · `get_task_steps` ·
`get_current_step` · `complete_step`

`get_context_for_task` sorts by importance and truncates to a token budget. It is
the hot path for prompt construction.

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
