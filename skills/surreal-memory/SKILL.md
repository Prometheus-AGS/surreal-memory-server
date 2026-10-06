---
name: surreal-memory
description: >
  Persistent cross-session memory via the surreal-memory-server MCP tools.
  Use at session start to recall prior context for the current project, when
  the user states a preference, decision, or fact worth keeping, after making
  a non-obvious technical decision, and before ending a session with
  unfinished work. Replaces per-conversation amnesia with scoped, searchable,
  graph-backed memory (SurrealDB + knowledge graph + semantic dedup).
---

# Surreal Memory

Cross-session memory for agent harnesses, backed by the surreal-memory-server
(MCP). Storage is durable, scoped, semantically deduplicated, and shared
across concurrent sessions — unlike per-session conversation context, it
survives `/clear`, compaction, restarts, and harness switches.

## The core loop: RECALL → WORK → REMEMBER

### 1. RECALL at session start (always, silently)

Before doing substantive work in a project, recover prior context:

1. `semantic_search` with a query built from the current task, e.g.
   `"<project-name> architecture decisions constraints in progress"`.
2. If the project has known entities, `search_entities` /
   `get_relations` for the project node to load people, components,
   and standing decisions.
3. Weigh results by recency; prefer scoped project memories over global ones.

Do this once, early, without being asked. Do not narrate the lookup unless
results materially change your plan — then say what you recalled and from
where.

### 2. WORK

Treat recalled memory as context, not ground truth: timestamps matter, code
changes, verify against the live repo before acting on stale recollections.

### 3. REMEMBER at decision points (sparingly, deliberately)

Store a memory when ANY of these happens:

- The user states a preference, convention, or standing instruction
  ("we never mock internal code", "deployments go to the Pi cluster").
- A non-obvious technical decision is made and WHY (the reasoning is the
  valuable part — the diff already records the what).
- A debugging session finds a root cause that will recur.
- Work pauses with unfinished state: what's done, what's next, what blocks.

Use `add_memory` with explicit scope:

| Scope | Use for | Example |
|---|---|---|
| `user` | Preferences, identity, cross-project habits | "Prefers terse replies, no summaries" |
| `project` (via project name/id) | Architecture decisions, conventions, state | "flint-gate uses receipt-driven ops; never infer success from timeouts" |
| `agent` | Agent-role-specific learnings | "As release-manager, always check Cross.toml first" |
| `session` | Ephemeral working state | "Currently mid-migration of table X" |

Semantic dedup is active: re-storing a near-duplicate updates rather than
piles up. Correct outdated facts with `update_memory` (keeps history) instead
of stacking contradictions.

## Knowledge graph for structured facts

Memories are for prose context; use the graph for structured relationships:

- `create_entity` for people, projects, components, concepts (with at least
  one observation).
- `create_relation` for directed facts: `Alice WORKS_ON flint-gate`,
  `surreal-memory-server DEPENDS_ON SurrealDB`.
- `add_observations` to extend entities as you learn more.
- `get_entity_history` / `get_graph_at_time` for temporal questions
  ("what did we know about X in August?").

Batch with `create_entities` / `create_relations` when loading several.

## What NOT to store

- Secrets, tokens, keys, credentials — never.
- Anything already recorded durably elsewhere: git history, AGENTS.md,
  CLAUDE.md, issue trackers. Memory is for what those don't capture:
  rationale, preferences, transient state, cross-session narrative.
- Raw tool output or long logs — distill first (one or two sentences).
- Trivia that will be irrelevant in a week.

## Session-end pattern

Before significant sessions end (or when asked to "remember this session"):
store one scoped memory: goal, outcome, open threads, next actions. Keep it
under ~200 words. Future sessions' RECALL step depends on this discipline.

## Operational notes

- Server: surreal-memory-server v1.10+ (MCP over streamable HTTP at
  `http://localhost:23001/mcp/http`; SSE and stdio also available).
- All writes go through a durable operation ledger — an acknowledged write
  is committed even if the connection drops afterward.
- Search is meaning-based (vector embeddings): phrase queries as natural
  language, not keywords. `search_entities` is exact-match only.
- `delete_all_memories` is scoped and destructive — confirm scope with the
  user before calling it.
