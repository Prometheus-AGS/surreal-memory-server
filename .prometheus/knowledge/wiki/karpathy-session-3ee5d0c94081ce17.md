---
type: SessionRecord
id: karpathy-session-3ee5d0c94081ce17
title: Karpathy session 3ee5d0c94081
tags:
- karpathy
- session-learning
sources:
- session:a529c600-97a5-4762-bd06-2049a9ec4fbd
timestamp: 2026-10-03T17:26:17.394188+00:00
created_at: 2026-10-03T17:26:17.394188+00:00
updated_at: 2026-10-03T17:26:17.394188+00:00
revision: 0
---

## Delta

I committed the fix, pushed it on `fix/30-stranded-operations`, and opened [PR #31](https://github.com/Prometheus-AGS/surreal-memory-server/pull/31), which closes #30. The full test run, including the three new tests, was still compiling when I opened the PR, so nothing is tested yet. Clippy passes with warnings treated as errors.

**Cause:** the list of unfinished operation states left out `planned` and `indexed`. Neither the startup check nor the rescan after each commit ever found operations stuck in those states, so a restart couldn't recover them. All 20 stuck operations in the issue are in one of those two states.

**What changed** (in `src/operations.rs`):
- The list now covers every unfinished state. A new test fails to compile if a state is added later without being classified, so it can't silently drift again.
- Every 60 seconds the server now re-checks all unfinished operations. One that failed on a temporary error now resumes without a restart.
- It now logs a warning when an operation record is missing, instead of skipping it silently.
- New tests:
  - Operations forced back to `planned` and `indexed` complete after a restart, and only the unfinished parts are embedded again.
  - An operation that fails once recovers without a restart.
  - The state-list check above.
- I added a one-line lesson to `docs/lessons.md`.

The other suggestions in the issue are not in this PR, as agreed: manual retry/reject endpoints, a metric for the oldest stuck operation, the embedding request-id mix-up, the "Specify a namespace to use" error and the cause of the 10-second database timeouts. The PR description lists them as follow-ups.

Once this is deployed and the server restarts, it should pick up the 20 stuck operations on its own. You can check each one with `GET /api/v2/operations/<id>`.

The PR shows no CI checks yet. I'll report the local test results when they finish, and I can turn on Auto-fix for CI failures on the PR if you want.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: a529c600-97a5-4762-bd06-2049a9ec4fbd
- Captured: 2026-10-03T17:24:28.088140Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- No changed paths detected.
