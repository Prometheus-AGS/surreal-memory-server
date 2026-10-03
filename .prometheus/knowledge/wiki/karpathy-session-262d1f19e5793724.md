---
type: SessionRecord
id: karpathy-session-262d1f19e5793724
title: Karpathy session 262d1f19e579
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T19:49:15.490265+00:00
created_at: 2026-09-26T19:49:15.490265+00:00
updated_at: 2026-09-26T19:49:15.490265+00:00
revision: 0
---

## Delta

The quality run is at the `clippy` stage (`fmt` passed). I'll apply fix A as soon as it finishes; I'm not editing `src/operations.rs` while it's being compiled.

**c8 diagnosis (memory-runtime):**
- **Root cause:** the operation ledger has the same defect c1 fixed for storage. All 11 ledger call sites in `src/operations.rs` clone the SDK session per call (`let db = connection.db.clone();`). In SDK 3.3.0, each clone must finish its attach, sign-in and `use` exchange, one acknowledged round trip at a time, before its query is sent. That's at least 3 serialized round trips per ledger call, where SDK 3.2.4 sent those steps without waiting.
- **Evidence it's an upgrade regression:** this test passed on the 3.2.4 lockfile, according to the archived ledger-recovery evidence.
- **The test failures:** after the four timeouts, nothing is left broken. The later calls just can't finish the replay within the 10 ms budget, so submit returns `503` and a receipt lookup returns `500`. Confidence is medium on this timing explanation, because the `503`/`500` response bodies weren't captured.
- **The live server:** partly explained. It runs an older build on the 3.2.4 lockfile, where the trigger is server slowness under heavy swap (lookups of 4–20 s). The same thing makes it worse, though: every timeout opens a brand-new WebSocket plus sign-in. Callers queued behind one slow reconnect all time out together.
- **A second bug:** the connection-setup comment around `surreal.rs:730` is wrong. In SDK 3.3.0, `query_timeout` isn't applied over WebSockets at all, so in server mode our outer timeout is the only deadline. A cancelled query also keeps running on the server.

**Fix A, which I'll apply once the quality run finishes:** 11 one-line changes in `src/operations.rs`, `&connection.db` instead of `.clone()`. It's the same change as c1, with no API, SQL or behaviour change. The ledger only uses bound variables and single-request transactions, so sharing a session is safe.

The check for fix A:
- `operation_query_deadline` passes 20 of 20 serial runs;
- the operations unit tests and `executor_recovery` pass;
- a server log shows one sign-in per ledger connection, not one per call;
- test failures print the response bodies, so any future `503`/`500` is diagnosable.

**Fix B needs your decision; I recommend it as a follow-up change.** Replace the ledger connection only on real connection errors, not whenever a query misses its deadline. That's what would stop the live server's timeout pile-ups, but it changes an archived spec (`operation-ledger-connection-recovery`) and its retry-once rule. So it needs storage review plus product and transports sign-off, and shouldn't be bundled into c8.

**Deploy:** fix A is small and addresses a cause of the live errors, so I'd hold the merge until it's in PR #27 too. Then one merge and one deploy covers c1, c2 and c8 (fix A).

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T19:49:12.371105Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .kbd-orchestrator/current-waypoint.json
- .kbd-orchestrator/phases/surrealdb-connection-architecture/progress.json
- .kbd-orchestrator/position-reminder.txt
- .kbd-orchestrator/position.json
- docker-compose-local.yml
- docker-compose.yaml
- docs/MEMPALACE.md
- test-surreal.sh
- .agent-team/memory-core/reviews/c1-c2-findings.md
- .agent-team/memory-core/reviews/c8-ledger-diagnosis.md
- .claude/worktrees/
- .kbd-orchestrator/phases/surrealdb-connection-architecture/children/
- crates/surreal-memory/.prometheus/
- history.txt
