---
type: SessionRecord
id: karpathy-session-81fad8217efc70fd
title: Karpathy session 81fad8217efc
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T15:38:32.139838+00:00
created_at: 2026-09-26T15:38:32.139838+00:00
updated_at: 2026-09-26T15:38:32.139838+00:00
revision: 0
---

## Delta

c2 is done and passes its check: the mixed-load failures are gone in both modes. It's committed and pushed on `feat/surrealdb-3x-connection-model` (`df77984`), and KBD has moved on to c3.

| Mixed 50/50 × 128 | Before c2 | After c2 (`d0caa4c`) |
|---|---|---|
| Server errors | 2,165–2,324 of 3,200 (`Connection reset`) | **0** |
| WebSocket connections per run | 20–24 | **1** |
| Embedded timeouts | 170–610 of 3,200 | **0** |
| Server p99 | 86–158 s | 36 s |

The results were recorded at a machine load of about 9, with the context in `load_repro_baseline.md`.

**Still worth knowing:**
- **Writes are still slow in server mode:** `add_memory × 64` p50 is about 5 s, versus about 1 s embedded. That points at the write path itself: the duplicate-check search plus the create and history writes on each insert. It isn't a connection problem.
- **Your live memory server doesn't have these fixes yet.** The launch agent runs the installed `/usr/local/bin/surreal-memory-server`, built before c1 and c2, so it still opens a session per call and still pulls the whole table on every search. Getting the benefit means merging this branch, then rebuilding and reinstalling that binary. I haven't done either.
- **c3's case is weaker now.** Routing all 44 call sites through the retry path was aimed mainly at the embedded timeouts, and those are gone. It would still give every operation consistent retries and deadlines, but no measured failure depends on it any more.

What next? My suggestion is to put c3 on hold and do c6 (correcting the "clone per task" docs, which are now actively misleading), then open a PR for c1 and c2 so the fixes can reach the live server.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T15:38:27.250532Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .kbd-orchestrator/phases/surrealdb-connection-architecture/progress.json
- .claude/worktrees/
- .kbd-orchestrator/phases/surrealdb-connection-architecture/children/
- crates/surreal-memory/.prometheus/
- history.txt
