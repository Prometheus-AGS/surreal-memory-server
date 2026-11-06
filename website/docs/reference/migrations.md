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
| 22 | `compression_derivations_and_claims` |
| 23 | `compression_evidence_and_memory_status` |
| 24 | `memory_fulltext_index` |

Versions 22 to 24 are additive:

- **22** adds the tables for summary derivations and their claims.
- **23** adds evidence rows and an optional `status` on `memory`, which marks archived
  sources as `summarized`.
- **24** defines the BM25 full-text index on `memory.content`. Before it, the BM25 leg
  of hybrid search queried an index that did not exist and hybrid search was silently
  vector-only.

Most migrations are SQL. Version 14 is a **repair** migration — it runs Rust
code to normalize legacy enum values written as single-key objects
(`{ Active: {} }`) into strings. It must precede v15, which tightens those
fields from `any` to `string`; the reverse order would reject the existing rows.

## Adding fields to a populated table

A fresh SurrealDB 4.0 database is created at `COMPATIBILITY 4`. On a table that
already has rows it accepts new `option<...>` fields but **rejects** changing a field
type, and rejects a non-optional field with a `DEFAULT`. Give new `memory` fields
`option<...>` types and treat `NONE` as the default state, instead of backfilling.

## Before you upgrade

Migrations run automatically on the first start of a new binary, so take an export
first. With the SurrealDB CLI:

```bash
surreal export --endpoint http://127.0.0.1:28000 --user root --pass root \
  --ns memory --db mcp memory-mcp-backup.surql
```

A startup log shows each step, for example:

```text
INFO Current schema version: v21
INFO Applying migration v22: compression_derivations_and_claims (checksum: 3bf98a8c)
INFO ✓ Migration v22 applied successfully
```
