---
type: SessionRecord
id: karpathy-session-a492bfe2b1be0945
title: Karpathy session a492bfe2b1be
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T14:55:01.645583+00:00
created_at: 2026-09-26T14:55:01.645583+00:00
updated_at: 2026-09-26T14:55:01.645583+00:00
revision: 0
---

## Delta

Still no cause for the resets yet. The traced run I set up couldn't show it, so I've rebuilt the harness and I'm running it again.

**Why the first traced run failed:**
- **No SDK trace:** `surrealdb` and `surrealdb-core` compile `tracing` with `release_max_level_debug`. That strips every `trace!` line from release builds, and the SDK logs its teardown reason only at `trace`.
- **No error samples:** the test harness hides the output of passing tests unless it's run with `--nocapture`. So missing error samples from that run proved nothing, and I don't know its error count. I briefly read it as "zero errors", which was wrong.

**A correction to the child phase assessment:** my conclusion that the extra WebSocket connections are reconnects isn't proven yet.
- **The count still happens:** this run showed 22 connections, as before.
- **The obvious other source is ruled out:** `operation_ledger_connection()`, which opens its own connection in server mode, is only called from the server binary, not the library the harness uses.
- **The trace will decide it:** whether the extra connections are resets or something else is exactly what the next run shows. I'll fix the assessment once I have that.

**What's running now:**
- **Build:** a debug build of the harness, so `trace!` isn't compiled out. It's slower, but timings don't matter for this run.
- **Run:** starts automatically when the build finishes. It uses `--nocapture`, the SDK's WebSocket module traced at `trace`, a fresh 3.3.0 scratch server at `debug`, and a one-hour limit.
- **Baseline:** this run's results are saved separately, and the committed baseline file is restored afterward.

**After it finishes,** I'll check the trace line before each `Reconnecting...` to see which of the four teardown causes fires. If there are no `Reconnecting...` lines, the extra connections have a different origin, and the error count from this run will show whether `Connection reset` still happens at all.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T14:52:34.921965Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- No changed paths detected.
