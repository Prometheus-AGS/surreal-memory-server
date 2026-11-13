---
type: SessionRecord
id: karpathy-session-127fc36aea2975bd
title: Karpathy session 127fc36aea29
tags:
- karpathy
- session-learning
sources:
- resource: session:cfc5bf77-8f81-45e6-975a-0283553c3962
generated:
  by: pk/1.11.0
  at: 2026-10-09T13:00:50.938860+00:00
created_at: 2026-10-09T13:00:50.938860+00:00
updated_at: 2026-10-09T13:00:50.938860+00:00
revision: 0
---

## Delta

The self-heal fix is implemented and committed locally as `170339a` on branch `fix/executor-metal-fault-self-heal` in `tools/surreal-memory-server`. Nothing is pushed, tagged, built into the installed binary, or pinned.

**What it does.** When the embedding child reports a Metal fault, the supervisor now:
- restarts the child pinned to CPU with `LOCAL_EMBEDDING_DEVICE=cpu`;
- replays the request on the new child;
- does the same for operation-scoped callers. They normally just see the failure, but the fault happens before any ledger write, so replay is safe.

All the code is in `src/executor.rs`. Before, a "Metal error" reply was classified as fatal and the poisoned child stayed alive.

**Verification.** One serialized integration run of the new test target and the existing recovery target: 2 new tests and 5 existing ones passed.
- The new tests (`tests/executor_metal_fault.rs`) drive a real child process through a fault-injecting fixture, covering both the interactive and operation-scoped paths.
- With the Metal detection temporarily disabled, both new tests failed, so they do catch the regression. I restored the fix and confirmed it.
- Only the files I touched are formatted. The full `cargo fmt --check` still flags `lib.rs` and `tests/query_embedding_latency.rs`, which I left alone.
- Not run: clippy, the full test suite, and a test against a real Metal GPU. The fault is simulated; I haven't reproduced the actual `MTLCompilerService` failure.

**Differences from the plan**
- I dropped the `operations.rs` backoff change. With the replay in place, an operation no longer pauses on this fault, and the repo's rules call for surgical changes.
- Any Metal error in the child, not just a poisoned lock, now demotes the daemon to CPU until it restarts. That seems right to me, but it is a policy choice.
- A panic that kills the child outright (rather than reporting `Failed`) still takes the existing restart path, which does not switch to CPU. I haven't seen whether the original panic surfaces as a `Failed` message or a child exit.

**Not done, and your call**
1. Push the branch and open a PR in the `surreal-memory-server` repo.
2. Tag v1.10.2 after that merges.
3. Bump the pin and `skill-system.json` to the new tag in this repo, then rebuild and re-sign the installed binary.

The host already runs on CPU through the launch-agent setting, so nothing is urgent. This fix matters for Apple Silicon machines that run `auto` or Metal. The repo's `docs/lessons.md` has no new entry, since no mistake happened here.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: cfc5bf77-8f81-45e6-975a-0283553c3962
- Captured: 2026-10-09T13:00:50.402943Z
- Project: /Users/gqadonis/Projects/prometheus/prometheus-skill-system/tools/surreal-memory-server

## Changed Paths

- No changed paths detected.
