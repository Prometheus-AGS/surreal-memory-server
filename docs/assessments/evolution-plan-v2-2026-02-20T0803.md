# Evolution Plan v2 — Extended: mem0 Parity + Mindmaps + Migration Safety
**Evolution Name:** `surreal-memory-server-baseline`
**Supersedes:** [evolution-plan-2026-02-20T0758.md](evolution-plan-2026-02-20T0758.md)
**Date:** 2026-02-20T08:03:00-06:00
**Iteration:** 3 (Extended Goals)

---

## Goals (Updated)

| # | Goal | Priority |
|---|------|----------|
| G1 | Crate library for embeddable memory (UAR integration) | P0 |
| G2 | A2A interface + REST API + HTTP/SSE + stdio transports | P1 |
| G3 | 50% competitive parity by reputation/use | P1 |
| G4 | Long-running named task memory + context management | P0 |
| **G5** | **Full mem0 feature parity** | **P1** |
| **G6** | **Mindmap creation for persona modeling and ideation** | **P1** |
| **G7** | **Zero-loss migration: current → next version** | **P0 (gate)** |

---

## G5 — mem0 Full Feature Parity

### What mem0 Is

mem0 is the most widely adopted memory framework for AI developers (41k+ GitHub stars, 107k+ platform visitors, 186M API calls/month Q3 2025, raised $24M YC round). It defines the **de-facto standard API surface** for agent memory.

### mem0 Feature Inventory

**Memory Type Model (4 types)**

| Type | Description | Example |
|---|---|---|
| **Episodic** | Event-specific, timestamped interactions | "User requested API docs in session on 2025-11-01" |
| **Semantic** | Extracted facts without event context | "User prefers TypeScript async/await" |
| **Procedural** | Behavioral patterns and execution preferences | "Always structure responses with headers first" |
| **Associative** | Relationships between memories and entities | "Project X relates to Team Y, depends on Library Z" |

**Scope Model (3 levels)**

| Level | Identifier | Description |
|---|---|---|
| User | `user_id` | Persists across all agents and sessions for a user |
| Session | `session_id` | Active conversation context — ephemeral unless promoted |
| Agent | `agent_id` | Per-agent persistent knowledge |

**Core API (what we must match)**

```
add(messages, user_id?, agent_id?, session_id?, metadata?, categories?) → Memory
search(query, user_id?, agent_id?, session_id?, limit?, filters?) → List[MemoryResult]
get(memory_id) → Memory
get_all(user_id?, agent_id?, session_id?) → List[Memory]
update(memory_id, data) → Memory
delete(memory_id) → void
delete_all(user_id?, agent_id?, session_id?) → void
history(memory_id) → List[MemoryHistory]   ← full change history per memory
```

**Memory Object Format (match exactly)**

```json
{
  "id": "uuid",
  "memory": "string (extracted fact/entity)",
  "user_id": "optional string",
  "agent_id": "optional string",
  "session_id": "optional string",
  "categories": ["health", "preferences", "work"],
  "metadata": { "arbitrary": "json" },
  "created_at": "ISO8601",
  "updated_at": "ISO8601",
  "score": 0.95
}
```

**Key Differentiating Features to Implement**

| Feature | Description | Implementation |
|---|---|---|
| **Auto-extraction** | LLM extracts facts automatically from raw conversation text | Add `add_from_conversation(messages)` tool that calls embedding provider to extract semantic memories |
| **Memory categories** | AI-assigned taxonomy tags (health, work, preferences, skills, etc.) | Store `categories: Vec<String>` on `Memory` struct; optionally auto-assign via LLM |
| **Memory compression** | Compress old memories into summaries to reduce token cost (up to 80% reduction per mem0) | `compress_memories(scope, older_than)` — summarize and replace |
| **Decay / TTL** | Memories automatically expire or decay in relevance weight | `valid_until: Option<Datetime>` + cron-style decay job |
| **Memory history** | Full version history of every memory change | `memory_history` table: `memory_id, version, old_value, new_value, changed_at` |
| **Semantic deduplication** | Prevent storing near-duplicate memories | On write: search similar at threshold 0.92; if match, update existing instead of inserting |
| **Self-improving** | Memory updates itself when contradictory information is added | Conflict detection on `add()` — if new memory contradicts existing (≥0.85 similarity), update rather than append |
| **Observability** | Track TTL, size, access count per memory | `access_count`, `last_accessed_at`, `size_bytes` fields on `Memory` |
| **Framework SDKs** | Python and JS SDK wrappers | Phase 3 — publish `surreal-memory-py` and `surreal-memory-js` thin API wrappers |

### mem0 MCP Tool Additions

```
add_memory(content, user_id?, agent_id?, session_id?, categories?, metadata?)
add_memories_from_conversation(messages, user_id?, agent_id?, session_id?)
search_memory(query, user_id?, agent_id?, session_id?, categories?, limit?)
get_memory(id)
get_all_memories(user_id?, agent_id?, session_id?)
update_memory(id, content)
delete_memory(id)
delete_all_memories(user_id?, agent_id?, session_id?)
get_memory_history(id)
```

### SurrealDB Schema Additions for mem0 Parity

```sql
-- Extend memory table
DEFINE FIELD memory_type ON memory TYPE string;   -- episodic|semantic|procedural|associative
DEFINE FIELD categories ON memory TYPE array<string>;
DEFINE FIELD categories.* ON memory TYPE string;
DEFINE FIELD metadata ON memory TYPE option<object>;
DEFINE FIELD session_id ON memory TYPE option<string>;
DEFINE FIELD access_count ON memory TYPE int DEFAULT 0;
DEFINE FIELD last_accessed_at ON memory TYPE option<datetime>;
DEFINE FIELD valid_until ON memory TYPE option<datetime>;
DEFINE FIELD version ON memory TYPE int DEFAULT 1;
DEFINE INDEX memory_session ON memory FIELDS session_id;
DEFINE INDEX memory_categories ON memory FIELDS categories;

-- Memory change history
DEFINE TABLE memory_history SCHEMAFULL;
DEFINE FIELD memory_id ON memory_history TYPE record<memory>;
DEFINE FIELD version ON memory_history TYPE int;
DEFINE FIELD old_content ON memory_history TYPE option<string>;
DEFINE FIELD new_content ON memory_history TYPE string;
DEFINE FIELD changed_at ON memory_history TYPE datetime;
DEFINE FIELD change_type ON memory_history TYPE string; -- created|updated|deleted
DEFINE INDEX memory_history_mem ON memory_history FIELDS memory_id;
```

---

## G6 — Mindmap Creation: Theory and Implementation

### Mindmap Theory (Research-Grounded)

Mindmaps as a structured representation technique have a rich theoretical history and multiple distinct forms. The following are the **dominant types** relevant to persona modeling and ideation with AI agents:

---

#### Type 1 — Radial (Tony Buzan) Mind Map
**Theory:** Coined by Tony Buzan in 1974, based on *radiant thinking* — the brain associates ideas outward from a central concept like neurons. Uses a central node, radiating branches, sub-branches, colors, and images.

**Best for:** Free-form brainstorming, neural association capture, rapid ideation, memory retention.

**Machine Representation:**
```json
{
  "type": "radial",
  "root": { "id": "root", "label": "Persona: Dr. Sarah Chen", "color": "#4a90e2" },
  "branches": [
    {
      "id": "b1", "label": "Values", "color": "#e24a4a",
      "children": [
        { "id": "b1a", "label": "Precision" },
        { "id": "b1b", "label": "Empathy" }
      ]
    }
  ]
}
```

---

#### Type 2 — Concept Map (Novak)
**Theory:** Developed by Joseph Novak at Cornell (1972), based on *constructivism* — knowledge is built by linking concepts with labeled connective phrases ("leads to", "requires", "contradicts"). Directional and propositional.

**Best for:** Domain modeling, structured knowledge capture, specification writing, understanding cause-effect chains.

**Machine Representation:**
```json
{
  "type": "concept",
  "nodes": [
    { "id": "n1", "label": "User Trust" },
    { "id": "n2", "label": "Consistent Memory" }
  ],
  "edges": [
    { "from": "n2", "to": "n1", "label": "builds", "directed": true }
  ]
}
```

---

#### Type 3 — Argument / Deliberation Map
**Theory:** Used in sensemaking and multi-agent research ideation (see Perspectra, 2025). Nodes represent claims, sub-claims, evidence, and rebuttals. Edges are `supports`, `contradicts`, `qualifies`.

**Best for:** Ideation with competing perspectives, debate structuring, analytical report generation, agent deliberation.

**Machine Representation:**
```json
{
  "type": "argument",
  "nodes": [
    { "id": "c1", "label": "Memory should expire after 90 days", "node_type": "claim" },
    { "id": "e1", "label": "Users return after long gaps (data)", "node_type": "evidence" },
    { "id": "r1", "label": "Some memories are permanent (identity)", "node_type": "rebuttal" }
  ],
  "edges": [
    { "from": "e1", "to": "c1", "label": "contradicts" },
    { "from": "r1", "to": "c1", "label": "qualifies" }
  ]
}
```

---

#### Type 4 — Hyperbolic / Hierarchical Tree Map
**Theory:** Hierarchical decomposition (org charts, WBS, ontologies). Non-radial — parent → children. Can be rendered as hyperbolic trees (Lamping, 1995) which scale to large hierarchies.

**Best for:** Organizational personas, feature decomposition, functional specifications, capability trees.

**Machine Representation:**
```json
{
  "type": "tree",
  "root": { "id": "r", "label": "Agent Capability Profile" },
  "children": {
    "r": ["n1", "n2"],
    "n1": ["n1a", "n1b"]
  },
  "nodes": { "n1": { "label": "Reasoning" }, "n1a": { "label": "Causal" } }
}
```

---

#### Type 5 — Temporal / Timeline Mind Map
**Theory:** Maps concept evolution over time. Each branch represents a time period; nodes within represent relevant ideas at that time. Particularly powerful combined with bi-temporal memory.

**Best for:** Project history, persona evolution over sessions, tracking how understanding of a topic changed.

**Machine Representation:**
```json
{
  "type": "temporal",
  "timeline": [
    {
      "period": "2025-Q1", "label": "Project Kickoff",
      "nodes": [{ "id": "t1a", "label": "Customer pain point identified" }]
    }
  ]
}
```

---

### Mindmap as a SurrealDB Entity

```sql
DEFINE TABLE mindmap SCHEMAFULL;
DEFINE FIELD name ON mindmap TYPE string;
DEFINE FIELD description ON mindmap TYPE option<string>;
DEFINE FIELD map_type ON mindmap TYPE string;   -- radial|concept|argument|tree|temporal
DEFINE FIELD root_node_id ON mindmap TYPE string;
DEFINE FIELD nodes ON mindmap TYPE array<object>;
DEFINE FIELD edges ON mindmap TYPE array<object>;
DEFINE FIELD agent_id ON mindmap TYPE option<string>;
DEFINE FIELD user_id ON mindmap TYPE option<string>;
DEFINE FIELD task_stream_id ON mindmap TYPE option<record<task_stream>>;
DEFINE FIELD tags ON mindmap TYPE array<string>;
DEFINE FIELD created_at ON mindmap TYPE datetime;
DEFINE FIELD updated_at ON mindmap TYPE datetime;
DEFINE INDEX mindmap_name ON mindmap FIELDS name, user_id;
```

### Mindmap MCP Tools

```
create_mindmap(name, map_type, root_label, description?, agent_id?, user_id?, task_stream_id?)
add_mindmap_node(mindmap_name, parent_id, label, node_type?, color?, metadata?)
add_mindmap_edge(mindmap_name, from_id, to_id, label?)
get_mindmap(name) → full graph JSON
list_mindmaps(user_id?, agent_id?, task_stream_id?)
delete_mindmap_node(mindmap_name, node_id)
export_mindmap(name, format) → JSON | Mermaid | DOT | Markdown

-- AI-powered
generate_persona_mindmap(user_id, name) → auto-builds radial persona map from memories
generate_ideation_mindmap(topic, map_type, context?) → AI-generates concept/argument map
```

### Persona Modeling via Mindmap + Memory Integration

The killer use case: **auto-generate a radial persona map from accumulated memories**.

```
1. search_memory(user_id=X, limit=50)   ← get all memories for user
2. Cluster by category: values, preferences, skills, relationships, goals, constraints
3. Build radial mindmap: root = user name/id, branches = categories, leaves = individual memories
4. Store as mindmap entity linked to user_id
5. As new memories arrive → update relevant branches incrementally
```

This gives agents a **real-time persona model** they can reference without scanning all raw memories.

### Ideation via Mindmap + Task Streams

For deep research or specification work:
1. Create a task stream named `"feature-x-research"`
2. As research proceeds, auto-add concept map nodes linking findings
3. Use the argument map type to capture competing perspectives
4. `get_context_for_task()` can include the mindmap structure as part of the context window

---

## G7 — Zero-Loss Migration Strategy

### The Problem

The current codebase stores entities and relations in SurrealDB with this schema:

```sql
-- CURRENT state (v1 — SurrealDB 2.0)
DEFINE TABLE entity SCHEMAFULL;
DEFINE FIELD name ON entity TYPE string;
DEFINE FIELD entity_type ON entity TYPE string;
DEFINE FIELD observations ON entity TYPE array<string>;
DEFINE FIELD created_at ON entity TYPE datetime;
DEFINE FIELD updated_at ON entity TYPE datetime;
DEFINE FIELD embedding ON entity TYPE option<array<float>>;
DEFINE INDEX entity_name ON entity FIELDS name UNIQUE;

DEFINE TABLE relation SCHEMAFULL;
DEFINE FIELD from ON relation TYPE string;
DEFINE FIELD to ON relation TYPE string;
DEFINE FIELD relation_type ON relation TYPE string;
DEFINE FIELD created_at ON relation TYPE datetime;
```

The next version adds: scoped memory table, task streams, mindmaps, memory history, mem0 fields, HNSW index. **None of this touches existing entity/relation data** — it's all additive.

### Migration Principle: Expand-Migrate-Contract

Use the **three-phase backward-compatible strategy**:

1. **Expand** — Add new fields/tables. Never remove or rename existing ones.
2. **Migrate** — Backfill old records with defaults for new optional fields.
3. **Contract** — Only after all data is migrated and verified, mark old unused paths as deprecated.

### Versioned Migration Runner

```rust
// crates/surreal-memory/src/storage/migrations/mod.rs
pub struct MigrationRunner {
    db: Surreal<Any>,
}

impl MigrationRunner {
    pub async fn run(&self) -> Result<()> {
        let current = self.get_schema_version().await?;
        for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
            tracing::info!("Applying migration v{}: {}", migration.version, migration.name);
            self.apply(migration).await?;
            self.set_schema_version(migration.version).await?;
        }
        Ok(())
    }
}

static MIGRATIONS: &[Migration] = &[
    Migration { version: 1, name: "initial_entity_relation_schema", up: migration_v1 },
    Migration { version: 2, name: "add_memory_table", up: migration_v2 },
    Migration { version: 3, name: "add_task_streams", up: migration_v3 },
    Migration { version: 4, name: "add_mindmaps", up: migration_v4 },
    Migration { version: 5, name: "add_mem0_fields", up: migration_v5 },
    Migration { version: 6, name: "add_hnsw_vector_index", up: migration_v6 },
];
```

### Schema Version Tracking

```sql
DEFINE TABLE schema_version SCHEMAFULL;
DEFINE FIELD version ON schema_version TYPE int;
DEFINE FIELD applied_at ON schema_version TYPE datetime;
DEFINE FIELD migration_name ON schema_version TYPE string;
DEFINE FIELD checksum ON schema_version TYPE string;
```

On startup, the runner reads the highest applied version and runs only pending migrations in order. **Idempotent** — safe to re-run.

### Migration v2 → v3 (Entity/Relation → memory_v1)

The existing `entity` table records are the *knowledge graph* layer. They are **NOT** being replaced — they stay as-is. The new `memory` table is additive.

For users upgrading who want to backfill existing observations as semantic memories:

```sql
-- Run once: backfill entities as semantic memories (optional, user opt-in)
FOR $e IN (SELECT * FROM entity) {
    INSERT INTO memory {
        id: rand::uuid(),
        content: string::join(" | ", $e.observations),
        scope: "global",
        memory_type: "semantic",
        categories: ["knowledge_graph"],
        tags: [$e.entity_type],
        embedding: $e.embedding,
        created_at: $e.created_at,
        updated_at: $e.updated_at,
        version: 1
    };
};
```

### Data Export / Backup Before Migration

```
surreal-memory-server export --format json --output ./backup-pre-migration.json
```

Exports all entities, relations, and (if present) memories to portable JSON. Import is the reverse:

```
surreal-memory-server import --from ./backup-pre-migration.json
```

The CLI export/import also enables **cross-version portability** — backup on v1, restore on v2.

### Migration Safety Rules

| Rule | Detail |
|---|---|
| **Never DELETE a field** | Mark as deprecated, keep reading it |
| **Never RENAME a field** | Add new field with new name, dual-write, then stop writing old |
| **Never change a field's type** | Add a new field with the correct type |
| **Always add new fields as `option<T>`** | Old records don't have them — `option` means they return `None` |
| **Always test migrations against a production-copy snapshot** | Run on `data/*.db` before shipping |
| **Every migration is idempotent** | Re-running causes no harm |
| **Embedded RocksDB DB files are preserved** | Migrations run IN-PLACE; no data is moved to new files |

---

## Updated Execution Phases (All 7 Goals)

### Phase 0 — SurrealDB 3.0 Unblock + Migration Infrastructure (~4 hours)
> **Gate for everything. Must be done first.**

| Action | Detail |
|---|---|
| A0.1 | Fix 15 SurrealDB 3.0 compile errors (`SurrealValue`, `RecordId`, `Datetime`) |
| A0.2 | Introduce `MigrationRunner` + `schema_version` table |
| A0.3 | Write `migration_v1` that codifies current schema as the baseline |
| A0.4 | `cargo build` exits 0; startup completes without data loss on existing `.db` files |

---

### Phase 1 — Library + Scoped Memory + mem0 Core + Migration v2-v3 (~3-4 days)
> Delivers **G1** (library crate), **G5 core** (mem0 API), **G4** (task streams), **G7** (migration)

| Action | Goal |
|---|---|
| A1.1 | Extract Cargo workspace: `crates/surreal-memory` library + `crates/surreal-memory-server` binary | G1 |
| A1.2 | Implement mem0 scope model: `user_id`, `session_id`, `agent_id`, `scope` on `Memory` | G5 |
| A1.3 | Implement 4 mem0 memory types enum: `Episodic`, `Semantic`, `Procedural`, `Associative` | G5 |
| A1.4 | Add `categories`, `metadata`, `version`, `access_count`, `valid_until` fields + migration | G5, G7 |
| A1.5 | Implement `memory_history` table + versioning on every write | G5 |
| A1.6 | Implement auto-deduplication on `add()` (similarity ≥ 0.92 → update, not insert) | G5 |
| A1.7 | Implement all 9 mem0 MCP tools (matching exact API surface) | G5 |
| A1.8 | Implement `TaskStream` entity + `add_to_task_stream`, `get_context_for_task` | G4 |
| A1.9 | Native HNSW vector index via SurrealQL `DEFINE INDEX … HNSW` (migration v6) | G3 |
| A1.10 | Batch `create_entities`, `create_relations` MCP tools | G3 |
| A1.11 | Write 25+ integration tests across all new tools | G1, G5 |
| A1.12 | UAR integration: align `Memory` domain struct with library | G1 |

---

### Phase 2 — Mindmaps + Transport + A2A + Hybrid Search (~1 week)
> Delivers **G6** (mindmaps), **G2** (A2A + REST + HTTP), remaining **G3** (parity), **G5 advanced**

| Action | Goal |
|---|---|
| A2.1 | `mindmap` table + all 5 map types + SurrealDB schema (migration v4) | G6 |
| A2.2 | 8 Mindmap MCP tools (`create_mindmap`, `add_node`, `add_edge`, `export_mindmap`, etc.) | G6 |
| A2.3 | `generate_persona_mindmap(user_id)` — auto-builds from memories | G6 |
| A2.4 | `generate_ideation_mindmap(topic, type, context)` — AI-structures a map | G6 |
| A2.5 | Mindmap export formats: JSON, Mermaid, Markdown | G6 |
| A2.6 | Axum REST API (`/api/v1/memory`, `/api/v1/entities`, `/api/v1/mindmaps`, `/api/v1/search`) | G2, G3 |
| A2.7 | HTTP/SSE MCP transport (enable `rmcp` server-side streamable HTTP) | G2 |
| A2.8 | A2A Agent Card + Task endpoints (`/agent.json`, `/a2a/tasks/*`) | G2 |
| A2.9 | Hybrid search: BM25 full-text + HNSW vector (scored merge) | G3 |
| A2.10 | mem0 Memory compression (`compress_memories`) + TTL decay job | G5 |
| A2.11 | `add_memories_from_conversation()` — auto-extract facts from raw chat messages using LLM | G5 |
| A2.12 | CI/CD GitHub Actions (build + lint + test) | G3 |

---

### Phase 3 — Advanced Context + Graph-RAG + SDKs (~2 weeks)
> Reaches 90%+ alignment; full production grade

| Action | Goal |
|---|---|
| A3.1 | Rolling task stream summarization (auto-compress when stream exceeds token budget) | G4 |
| A3.2 | Model context profiles registry (GPT-4o, Claude, Gemini token profiles) | G4 |
| A3.3 | Graph traversal tools: `find_path`, `expand_neighbors`, `get_related` | G3 |
| A3.4 | Migrate embedded from RocksDB → SurrealKV | G1, G3 |
| A3.5 | Publish `surreal-memory` crate to crates.io | G1 |
| A3.6 | Python SDK wrapper (`surreal-memory-py`) | G5 |
| A3.7 | JavaScript/TypeScript SDK wrapper | G5 |
| A3.8 | Multi-tenant namespacing per user/org | G2 |
| A3.9 | Temporal entity history queries (`get_entity_history`, `get_graph_at_time`) | G3 |
| A3.10 | Persona mindmap auto-update on every new memory write | G6 |

---

## Updated Alignment Projections

| Phase Completed | G1 | G2 | G3 | G4 | G5 | G6 | G7 | Overall |
|---|---|---|---|---|---|---|---|---|
| Baseline (now) | 0% | 0% | 25% | 0% | 0% | 0% | 0% | **4%** |
| After Phase 0 | 10% | 0% | 25% | 0% | 0% | 0% | 60% | **18%** |
| After Phase 1 | 80% | 0% | 55% | 75% | 70% | 0% | 95% | **59%** |
| After Phase 2 | 85% | 85% | 80% | 80% | 85% | 80% | 98% | **79%** |
| After Phase 3 | 95% | 92% | 90% | 95% | 95% | 90% | 100% | **93%** ← terminate |

---

## Risk Register (Updated)

| Risk | Severity | Mitigation |
|---|---|---|
| SurrealDB 3.0 beta API churn | High | Pin to 3.0.0, monitor release notes |
| Migration corrupts embedded RocksDB data | Critical | Backup before every migration; dry-run mode |
| Auto-deduplication false positives (0.92 threshold) | Medium | Make threshold configurable per scope |
| LLM cost for auto-extraction / persona generation | Medium | Batch extraction; cache embedding results; make opt-in |
| A2A spec changes (v0.3 → v1.0 finalization) | Medium | Version A2A endpoints; graceful degradation |
| Mindmap JSON schema evolves (new map types) | Low | Use flexible `object` type in SurrealDB for node/edge data |
| Python/JS SDKs lag behind Rust core | Low | Auto-generate from OpenAPI spec |

---

*Plan v2 generated by PMPO Iterative Evolver — Iteration 3. Supersedes evolution-plan-2026-02-20T0758.md.*
*State updated: `.evolver/evolutions/surreal-memory-server-baseline/state.json`*
