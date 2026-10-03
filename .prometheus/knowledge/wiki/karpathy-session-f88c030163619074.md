---
type: SessionRecord
id: karpathy-session-f88c030163619074
title: Karpathy session f88c03016361
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T17:23:45.852206+00:00
created_at: 2026-09-26T17:23:45.852206+00:00
updated_at: 2026-09-26T17:23:45.852206+00:00
revision: 0
---

## Delta

Every client connected right now reaches port 23001 from this machine. Here are my three recommendations.

## 1. Add the W2 integration test before merging: **Yes**

**Why:** c2 changes how memory search works. It moves from an exact scan of every memory to an approximate index lookup. That change feeds duplicate detection in `add_memory` and `add_to_task_stream`, which write to your live database.
- **What we have so far:** the only execution evidence for c2 is the load test, which uses all-zero vectors and a single scope. It proves the resets are gone. It doesn't show that search returns the right memories, in the right order, for the right scope, or that duplicates are still caught.
- **Why not skip it:** a regression here fails silently. You'd get duplicate memories or slightly different recall, with no error anywhere. The load test can't catch that.

**What the test covers:** fixed non-zero vectors across several scopes, plus an unscoped case, compared against the old exact ranking. It also checks that a near-duplicate is still caught at 0.92 similarity. It runs against a scratch 3.3.0 server. It's about 30 minutes to write, plus a build, and delays the merge by roughly an hour.

**Skip it only if** getting the fix live today matters more than that risk. You could ship now and add the test right after, since duplicate memories are cleanup work, not data loss.

## 2. Fix the older `fmt` drift in a separate commit: **Yes**

**Why:**
- It's 4 files from the 09-23 scoped-credentials commits. Running `cargo fmt` changes whitespace and line breaks only, with no behaviour change, and a reviewer can see that at a glance in a separate commit.
- `CLAUDE.md` makes `cargo fmt --check` part of the required quality check before commits. Leaving it failing means every future change either fails that check or learns to ignore it. The second outcome is how the "compiled, so it's done" habit crept into this repo.
- Keeping it in its own commit keeps c1/c2 easy to review and easy to revert.

The only argument against is that it touches files outside this phase's scope. Formatting-only changes carry no real risk, so I'd do it.

## 3. Port 23001 binding: **Move it to `127.0.0.1`**

**Why:**
- **Evidence:** all 7 live connections come from `127.0.0.1` (Claude Code and Codex). Nothing reaches it over the network.
- **Exposure today:** it listens on every interface, including `10.0.0.20` on your LAN and several `192.168.x` bridges (OrbStack/VM networks). The reviewer found no visible authentication on the REST or MCP endpoints.
- **Risk:** any device on your network, or any container on those bridges, could read, write or delete your memories. That's a much larger exposure than the benefit, and your own `settings.json` already describes this service as "loopback only".
- **How:** one environment variable in `ai.prometheus.surreal-memory-native.plist`, if the server supports a bind address. Otherwise a small config change. It's applied as part of the deploy step, which needs a restart anyway, so there's no extra downtime.

**Caveat:** if a Docker or OrbStack container, or another machine, needs to reach it, loopback would break that. Nothing is doing so right now, but a service that only runs sometimes, such as a container The Boss or UAR starts on demand, wouldn't show up in this snapshot. If you know of one, keep `0.0.0.0` and add authentication instead. That's a bigger job for memory-transports.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T17:23:37.369556Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .kbd-orchestrator/current-waypoint.json
- .kbd-orchestrator/phases/surrealdb-connection-architecture/progress.json
- .kbd-orchestrator/position-reminder.txt
- .kbd-orchestrator/position.json
- src/operations.rs
- .agent-team/memory-core/reviews/c1-c2-findings.md
- .claude/worktrees/
- .kbd-orchestrator/phases/surrealdb-connection-architecture/children/
- crates/surreal-memory/.prometheus/
- history.txt
