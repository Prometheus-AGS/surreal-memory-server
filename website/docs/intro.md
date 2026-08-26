---
id: intro
title: Introduction
sidebar_position: 1
---

# Surreal Memory Server

Durable agent memory over SurrealDB, exposed through the Model Context Protocol.

The project ships **two artifacts** from one workspace:

| Artifact | Path | Consumers |
|---|---|---|
| **Binary** | `src/main.rs` | MCP clients over stdio or HTTP |
| **Library** | `crates/surreal-memory/` | `universal-agent-runtime`, `uar-wisc` |

The library is the more important of the two. The binary is a thin MCP shell
around it; the library is a direct dependency of the Universal Agent Runtime, so
its `MemoryStorage` trait is a published interface rather than an internal
detail.

## What problem it solves

Agents lose everything between turns. Naive fixes — dumping transcripts into a
vector store, or stuffing history into the prompt — fail in predictable ways:
retrieval returns the most recently written text rather than the most relevant,
nothing distinguishes a durable fact from a passing remark, and there is no way
to scope one user's memory away from another's.

This server treats memory as **typed, scoped, and durable** rather than as a
bag of embeddings:

- **Typed** — a decision, an error resolution, and a general fact are different
  kinds of memory with different retention and retrieval behavior.
- **Scoped** — every memory belongs to a global, agent, user, session, or task
  scope, and reads are filtered by that scope rather than by convention.
- **Durable** — writes go through a migration-governed SCHEMAFULL schema, with
  an audit trail, so a memory's history is reconstructable.

## Capabilities

- **Scoped memory** with a mem0-compatible surface
- **Knowledge graph** of entities and relations, with Graph-RAG traversal
- **Hybrid search** combining BM25 full-text and HNSW vector indexes via
  reciprocal rank fusion
- **TaskStreams** — token-budgeted context windows for long-running work
- **Mindmaps** — structured graphs with export
- **Memory Palace** (optional `palace` feature) — a second 384-dimension
  retrieval space

The MCP surface exposes **59 tools** across these areas.

## Where to start

- [Architecture overview](architecture/overview.md) — how the pieces fit
- [Memory model](architecture/memory-model.md) — scopes and types
- [Deployment](operations/deployment.md) — running it
- [Design decisions](design-decisions.md) — why it is built this way
