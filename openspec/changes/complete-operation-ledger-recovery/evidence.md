# Verification evidence

Date: 2026-09-21

## Behavior

- Server-mode operation-ledger initialization opens a separately authenticated
  transport before the first query; embedded mode clones its in-process handle
  and does not reopen RocksDB.
- Initialization and replacement share one async mutex and check the published
  generation after acquiring it, so overlapping stale callers publish only one
  next generation.
- A database deadline is retryable only when replacement succeeded or another
  caller already published a newer generation. Replacement failure preserves
  the existing API error but is not classified as recovered.
- Startup discovery and the coordinator drain each retry at most once only for
  that typed recovered condition. Executor and ordinary errors are not retried.

## Local verification

- `cargo fmt --all --check` — exit 0.
- `git diff --check` — exit 0.
- `RUSTC_WRAPPER= cargo check --locked --package surreal-memory-server --no-default-features --features server-only`
  — exit 0.
- `RUSTC_WRAPPER= cargo test --locked --lib operations::tests:: -- --nocapture`
  — exit 0; 17 passed, 0 failed.
- `RUSTC_WRAPPER= cargo test --locked --test operation_query_deadline --no-default-features --features server-only -- --nocapture`
  — exit 0; 1 passed, 0 failed. The real isolated server kept general storage
  responsive while four ledger receipt queries timed out, then the same
  production coordinator committed later work.
- `RUSTC_WRAPPER= cargo test --locked --test executor_recovery -- --nocapture`
  — exit 0; 4 passed, 0 failed.
- `openspec validate complete-operation-ledger-recovery --strict` — exit 0.
- `openspec validate bound-operation-query-deadlines --strict` — exit 0.
- `openspec validate bound-operation-reconciliation-projection --strict` —
  exit 0.
- `prometheus-rust-auditor format`, `enforce`, and `inventory` — exit 0; no
  findings. `partition` exited 0 with six informational AI-loop-pending rows.
- `prometheus-rust-auditor deps` could not pass because `cargo-deny` is absent
  and the committed lockfile already contains the same three `cargo audit`
  advisories as the working lockfile: RUSTSEC-2026-0235, RUSTSEC-2023-0071,
  and RUSTSEC-2026-0285. This change adds only the already workspace-pinned
  `arc-swap` package dependency and introduces none of those advisory paths.

## Isolated review

- Round 1: BLOCK. It required exercising the actual coordinator drain retry
  branch and removing the duplicate mutation to the prior deadline spec.
- Repairs: added a deterministic `drain_pending` regression for recovered,
  repeated, and executor errors; restored the prior change's spec to committed
  bytes.
- Round 2: PASS. No blocking findings.

## Publication

- Pull request: https://github.com/Prometheus-AGS/surreal-memory-server/pull/21
- Source commits: `b844b8e` and `406200e`
- Merge commit: `be198125eb27fb06adc2194037b48ed254bcbfb1`
- PR state: MERGED at 2026-09-21T04:57:37Z.

## Deployment state

The repaired source is not yet the installed binary. Deployment certification
requires the merged commit to be built, signed, installed at both owned paths,
and observed draining every accepted receipt before `prometheus doctor --json`
can pass.

## Installed-runtime follow-up

The first installed build remained ready but its startup reconciliation timed
out twice at the 10-second discovery deadline. Direct HTTP measurements against
the same database showed the cause: the `state NOT IN [...]` discovery query
took 8.74 seconds, while separate state-index equality queries took 1.20–2.22
seconds each. The coordinator also repeated the full discovery query after
every commit, making backlog recovery quadratic.

The follow-up implementation queries `accepted`, `validated`, `blocked`, and
`processing` separately under the configured per-query deadline, then sorts
and deduplicates the combined identities. The drain loop now rescans once after
a wave that committed work, preserving dependency recovery without one full
ledger scan per committed operation.

- `cargo fmt --all --check` and `git diff --check` — exit 0.
- `openspec validate complete-operation-ledger-recovery --strict` — exit 0.
- `openspec validate bound-operation-query-deadlines --strict` — exit 0.
- `openspec validate bound-operation-reconciliation-projection --strict` —
  exit 0.
- `RUSTC_WRAPPER= cargo test --locked --test operation_query_deadline --no-default-features --features server-only -- --nocapture`
  — exit 0; 2 passed, 0 failed. The added real-server case proves an
  earlier-sorted dependent operation commits in a second drain wave after its
  later-sorted prerequisite commits.
- The first run of that integration target produced 1 pass and 1 failure
  because the new fixture used literal record keys instead of the production
  SHA-256 key derivation. Correcting the fixture made the unchanged production
  path pass.
- `RUSTC_WRAPPER= cargo test --locked --test executor_recovery -- --nocapture`
  — exit 0; 4 passed, 0 failed.
- `prometheus-rust-auditor enforce`, `format`, and `inventory` — exit 0 with
  no findings. `partition` exited 0 with the same six informational
  AI-loop-pending rows.

The task's isolated review budget was already consumed by the two recorded
rounds, ending in PASS. The installed-runtime regression therefore proceeds to
the same local gates and a fresh deployment proof rather than a third critic
round. Task 3.1 remains open until the new build drains accepted receipts to
zero and `prometheus doctor --json` exits successfully.
