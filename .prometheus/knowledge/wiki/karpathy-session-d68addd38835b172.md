---
type: SessionRecord
id: karpathy-session-d68addd38835b172
title: Karpathy session d68addd38835
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T15:30:00.876382+00:00
created_at: 2026-09-26T15:30:00.876382+00:00
updated_at: 2026-09-26T15:30:00.876382+00:00
revision: 0
---

## Delta

I've found why the mixed load gets `Connection reset` errors and committed a fix. The run that checks it is in progress.

**Root cause:**
- **The error:** every one of the 19 reconnects in the traced run came right after `Message too long: ~67.9 MB > 67108864`. A single query response was over the SDK's 64 MiB WebSocket limit. The SDK treats that as a broken socket: it fails every request still waiting with `Connection reset`, then reconnects.
- **The oversized response:** it comes from `search_memories`. That function never used the vector index. It downloaded every memory in scope, full embeddings included, and ranked them in Rust.
- **Why every operation paid for it:** both `hybrid_search_memories` and `add_memory`'s duplicate check call it, so every search and every insert pulled the whole table. At 1,536 dimensions that crosses 64 MiB at around 5,000 memories, which the test only reaches in its mixed phase.
- **Your live data:** `memory/mcp` (3,281 memories at 384 dimensions) is already moving about 13 MB per search and heading toward the same limit.

**The fix** (`d0caa4c`, pushed on `feat/surrealdb-3x-connection-model`):
- `search_memories` now runs one nearest-neighbour query on the existing `memory_embedding_hnsw` index. The same user/agent/session filters are applied during the search, so only the top `limit` rows come back. Those few rows are still sorted with the existing Rust cosine code, so ordering stays exact.
- Only `search_memories` changed. The public `MemoryStorage` interface is untouched.
- It compiles cleanly with palace included.

**One behaviour change to know about:** vector search now comes from the HNSW index, which is approximate, instead of a brute-force scan of every memory. The duplicate check at 0.92 similarity should behave the same in practice. In rare cases a near-identical memory could be missed if the index doesn't return it among the top 5.

**Also on the branch** (`5d39810` and `e12e90e`):
- Opt-in SDK tracing for the load test, test code only.
- The root cause, recorded in the child phase assessment.

I also removed a second stale git lock (from 10:13) along the way.

**Check in progress:** one release run of both modes on a fresh 3.3.0 scratch server with debug logging. Machine load is down to about 9. To pass, server mode needs 0 `connection` errors under mixed load and one WebSocket per run. If it passes, I'll mark c2 complete in KBD and move on to c3.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T15:30:00.266849Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .kbd-orchestrator/phases/surrealdb-connection-architecture/progress.json
- .claude/worktrees/
- .kbd-orchestrator/phases/surrealdb-connection-architecture/children/
- crates/surreal-memory/.prometheus/
- history.txt
