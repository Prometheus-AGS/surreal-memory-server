---
type: SessionRecord
id: karpathy-session-16fdc5824478f386
title: Karpathy session 16fdc5824478
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T14:24:46.835354+00:00
created_at: 2026-09-26T14:24:46.835354+00:00
updated_at: 2026-09-26T14:24:46.835354+00:00
revision: 0
---

## Delta

The child phase `surrealdb-3x-connection-model` is set up and active under the connection-architecture phase, and committed as `051d8a7`. KBD points at its next change, c2 (the WebSocket resets).

**Was the verification run necessary? Yes.** A full run would have been redundant once the child phase existed, but this one answered the key question in one server round, and I stopped the unneeded embedded half of round 2.
- **Sharing one session is a real, large win:** sign-ins per run dropped from about 2,700 to 24, and search p50 in server mode went from 2.3–3.0 s to 14 ms.
- **It didn't fix the failures:** mixed-load `Connection reset` errors stayed at about 2,200 of 3,200.
- **Resets now counted:** the server saw 24 WebSocket connections per run, one long-lived and 23 short-lived. So the socket is being torn down about 23 times per run, and the server logs each close as normal.

Without the run, the child phase would have started from my wrong hypothesis that session churn caused the resets. Instead it starts with the resets as the top unexplained problem.

**What's in the child phase** (`assessment.md` and `plan.md` under `.kbd-orchestrator/phases/surrealdb-3x-connection-model/`):
- **Rules:**
  - 3.3.0 only;
  - load tests run on fresh scratch servers, never the shared `:28000`;
  - a change is complete only when its load-test pass/fail check is met on its own commit, not when it compiles.
- **Changes, in order:**
  1. **c1 shared session:** complete (`de3582e`).
  2. **c2 reset root cause (next):** pass/fail is 0 connection errors and one WebSocket per run.
  3. **c3 retry coverage:** route all 44 direct call sites through the retry path, the operation deadline and the embedded concurrency limit.
  4. **c4 palace session fix:** needs your sign-off first, because it changes a public API.
  5. **c5 session-pool decision:** made from the load-test data plus your vertical-versus-horizontal answer.
  6. **c6 docs:** correct the wrong "clone per task" guidance.
  7. **c7 closing evidence:** a full load-test run on a quiet machine.
- **Parent phase:** its change-8 and change-5 were moved into the child. They show as cancelled in the parent, not lost.

**Still needs your attention:** the Codex session that committed `de3582e` and `d3b0bb3` is still active in this checkout. Before I start c2, can you pause it, or confirm it isn't touching the storage code?

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T14:10:18.905156Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- No changed paths detected.
