---
id: migrations
title: Migrations
sidebar_position: 3
---

# Migrations

Migrations live in `crates/surreal-memory/src/storage/migrations/mod.rs` and are
applied automatically on startup, recorded in `schema_version` with a checksum.

## The schema/struct sync rule

**Every persisted field needs a matching `DEFINE FIELD`.** SCHEMAFULL tables
reject anything undeclared, and this has caused production failures — for
example `TaskStream.auto_summarize` was added to the struct without a migration
and `create_task_stream` failed at runtime.

Adding a field:

1. Add it to the struct with `#[serde(default)]` for backward compatibility.
2. Add a migration **in the same change**, not a follow-up.
3. Test in both embedded and server mode.

## Current migrations

| Version | Name |
|---|---|
| 1 | `initial_entity_relation_schema` |
| 2 | `scoped_memory_table` |
| 3 | `task_stream_table` |
| 4 | `memory_history_table` |
| 5 | `hnsw_vector_indexes` |
| 6 | `mindmap_table_and_fulltext_indexes` |
| 7 | `task_stream_auto_summarization_fields` |
| 8 | `memory_metadata_flexible` |
| 9–13 | Mindmap schema refinements |
| 14 | `legacy_enum_string_normalization` (repair) |
| 15 | `enum_fields_as_strings` |
| 16 | `palace_drawers_table` |
| 17 | `dynamic_embedding_index_metadata` |
| 18 | `task_stream_scope_unique_index` |
| 19 | `task_step_table` |
| 20 | `durable_operation_ledger` |
| 21 | `embedding_executor_journal` |

Most migrations are SQL. Version 14 is a **repair** migration — it runs Rust
code to normalize legacy enum values written as single-key objects
(`{ Active: {} }`) into strings. It must precede v15, which tightens those
fields from `any` to `string`; the reverse order would reject the existing rows.
