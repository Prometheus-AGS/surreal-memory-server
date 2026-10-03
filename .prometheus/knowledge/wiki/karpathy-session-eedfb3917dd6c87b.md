---
type: SessionRecord
id: karpathy-session-eedfb3917dd6c87b
title: Karpathy session eedfb3917dd6
tags:
- karpathy
- session-learning
sources:
- session:7986632d-920d-4490-a3f5-cfa3add13821
timestamp: 2026-09-26T13:49:54.505275+00:00
created_at: 2026-09-26T13:49:54.505275+00:00
updated_at: 2026-09-26T13:49:54.505275+00:00
revision: 0
---

## Delta

I haven't committed change-8 myself, but it is already committed. A Codex session working in this same checkout committed my uncommitted edit as `de3582e` ("fix(storage): reuse the active SurrealDB session"), under a Codex attribution. It had just committed the KBD state and the OpenSpec archive moves too (`d3b0bb3`). The content of `de3582e` matches my edit exactly, so nothing was lost. But two agents are writing to the same working tree, which risks clobbering each other's work. I haven't touched Codex's commits. Can you check what that session is doing, or pause it while change-8 finishes?

**Change-8 changed scope, based on evidence.** Reconnects weren't the problem:
- **What the server log showed:** in the diagnostic run, every query on the one WebSocket came with its own attach, sign-in, `use` and detach (about 2,700 of each). So every storage call was opening and tearing down a full server-side session.
- **Why:** in SDK 3.x, `Surreal::clone()` creates a new session (`surrealdb-3.3.0/src/lib.rs:337`), and dropping it detaches that session. `live_db()` cloned the handle on every call, across all 44 call sites.
- **The fix:** the shared connection is now held as an `Arc<Surreal<Any>>`, and `live_db()` hands out that `Arc` without cloning the SDK handle. All operations share the one session set up at connect time; nothing changes session state later, so sharing is safe.
- **Public API unchanged:** `db()` and the palace adapter still return owned handles, now documented as opening a session per call. So nothing changes for UAR.
- **Compiles:** `cargo check -p surreal-memory --all-targets --features palace` is clean.
- **Verification:** two rounds, server and embedded mode, on a fresh 3.3.0 scratch server with debug logging (about 30 minutes). It counts as passing if sign-ins drop from one per operation to about one per run, and the mixed-load `Connection reset` errors drop sharply.

**Two more findings, not fixed in this change:**
1. **Most operations bypass the retry wrapper.** Only one internal helper uses `retry_operation`. The other 43 call sites go straight to `live_db()`, so they get no retry, no operation deadline and no embedded concurrency limit. The embedded 10 s timeouts are partly this.
2. **The palace path still opens a session per operation,** because its public closure returns an owned handle. Fixing it would change a public signature, so it's deferred.

The repo's own guidance ("`Surreal<Any>` is clone-safe, clone per task" in `CLAUDE.md` and `docs/lessons.md`) is now wrong for 3.x. I'll correct it when I close the change.

**The decision you need to make: deployment shape** (assessment §6.1, which gates change-5). How should `surreal-memory-server` handle more load?

- **Vertical:** one server process takes all the concurrent traffic. Change-5 then gets built: a small pool of sessions so heavy writes stop slowing reads on the one connection.
- **Horizontal:** several server processes run behind a load balancer, sharing one SurrealDB server. Change-5 becomes documentation only (`docs/DEPLOYMENT.md`), and each process keeps a single shared connection.

My recommendation is to wait for this run's numbers. If sharing one session fixes the mixed-load failures, the session pool may be unnecessary for either answer. Then the question is only about how you deploy: today it's one launchd process on this machine.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 7986632d-920d-4490-a3f5-cfa3add13821
- Captured: 2026-09-26T13:48:54.535773Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .claude/worktrees/
- crates/surreal-memory/.prometheus/
- history.txt
