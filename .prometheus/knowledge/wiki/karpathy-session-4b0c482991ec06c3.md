---
type: SessionRecord
id: karpathy-session-4b0c482991ec06c3
title: Karpathy session 4b0c482991ec
tags:
- karpathy
- session-learning
sources:
- resource: session:a529c600-97a5-4762-bd06-2049a9ec4fbd
generated:
  by: pk/1.9.0
  at: 2026-10-03T20:16:04.957655+00:00
created_at: 2026-10-03T20:16:04.957655+00:00
updated_at: 2026-10-03T20:16:04.957655+00:00
revision: 0
---

## Delta

#36 is fixed and closed. The 10-second ledger timeouts came from macOS scheduling, not from the memory server's code. Both LaunchAgents were set to `ProcessType = Background`, so launchd ran the database and the memory server at the lowest scheduler priority (4 instead of the normal 31), with throttled disk and network I/O. This Mac is under heavy memory pressure (about 20 GB in the compressor, 13 GB of swap, ~73 MB free), and at that priority single-row database queries regularly took several seconds. That also explains why fresh connections themselves kept timing out (1,479 of them) and why SurrealDB's own 60-second internal heartbeat was timing out.

**How I confirmed it:** I ran two scratch copies of the same SurrealDB 3.3.0 binary side by side, one at normal priority and one at background priority, and timed 150 single-row writes on each.

| Priority | p50 | p99 | max |
|---|---|---|---|
| normal | 16 ms | 22 ms | 53 ms |
| background (how the services ran) | 212–252 ms | 6.8–8.6 s | 12.5–14.7 s |

**The fix:** [prometheus-skill-system#120](https://github.com/Prometheus-AGS/prometheus-skill-system/pull/120) sets both service templates to `ProcessType = Standard`. It's merged, the plists are re-rendered, and both services were reloaded. They now run as daemons at priority 20. Only the priority line changed in each plist, and the old versions are saved as `*.bak-pre-standard-*`.

**Live results on this machine:**

| | before | after |
|---|---|---|
| single-row database read (p50 / p99 / max) | 401 ms / 6.3 s / 9.1 s | 16.7 ms / 22–25 ms / 51 ms |
| single-row database write (p50 / p99) | 637 ms / 6.2 s | 16.6 ms / 19–63 ms |
| durable `add_memory`, end to end | ~3 s | ~0.8 s, now mostly embedding time |
| hybrid search | not measured | 30–80 ms |
| ledger errors or timeouts since the reload | — | 0 |

[#42](https://github.com/Prometheus-AGS/surreal-memory-server/pull/42) records why the services must never run at `Background`, and how to check it, in the deployment doc and `docs/lessons.md`. The ledger hardening from #37 stays in place.

**Worth knowing:**
- **The underlying memory pressure is still there.** The services now get proper scheduling, but the Mac remains heavily swapped. OrbStack alone holds about 3.9 GB. If latency creeps back, freeing memory is the next lever.
- **Fewer executor restarts likely.** The MLX embedding child runs under the memory server, so it now also gets normal priority. That should cut the "nonresponsive executor" restarts, though I haven't measured them.
- **Priority was the cause.** The "probe before replacing" idea from #37, which I rejected, was aimed at the wrong problem.
- **About a minute of downtime.** Both services restarted once each for this change.
- **Every issue from this session is closed:** #30 and #32 through #36.
- **Your checkout is still untouched.** Your `surreal-memory-server` checkout is still on `fix/34-executor-cancel-safety` with your uncommitted files.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: a529c600-97a5-4762-bd06-2049a9ec4fbd
- Captured: 2026-10-03T20:16:04.048957Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .agents/skills/.openspec-target
- .agents/skills/openspec-apply-change/
- .agents/skills/openspec-archive-change/
- .agents/skills/openspec-bulk-archive-change/
- .agents/skills/openspec-continue-change/
- .agents/skills/openspec-explore/
- .agents/skills/openspec-ff-change/
- .agents/skills/openspec-new-change/
- .agents/skills/openspec-onboard/
- .agents/skills/openspec-propose/
- .agents/skills/openspec-sync-specs/
- .agents/skills/openspec-update-change/
- .agents/skills/openspec-verify-change/
- .agents/workflows/
- .claude/commands/opsx/propose.md
- .claude/commands/opsx/update.md
- .claude/skills/openspec-propose/
- .claude/skills/openspec-update-change/
- .kbd-orchestrator/phases/surrealdb-connection-architecture/children/
- .opencode/commands/opsx-propose.md
- .opencode/commands/opsx-update.md
- .opencode/skills/openspec-propose/
- .opencode/skills/openspec-update-change/
- crates/surreal-memory/.prometheus/
- history.txt
