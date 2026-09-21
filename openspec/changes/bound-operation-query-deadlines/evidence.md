# Verification evidence

Date: 2026-09-21

## Source and review

- PR #20: https://github.com/Prometheus-AGS/surreal-memory-server/pull/20
  - source: `93372f820438dc485277530e8d5b23c21711090d`
  - merge: `b000bced73f4f626625b4379d26cda657999e080`
- Critic round 1 returned BLOCK because the recovery assertion used a second
  coordinator. The regression was repaired to use the production coordinator.
- Critic round 2 returned BLOCK because the fixture could race with coordinator
  processing. The terminal receipt is now seeded before the coordinator starts.
- Follow-up change `complete-operation-ledger-recovery` addressed the recorded
  blocker in a new review cycle. Its round 1 critic required a direct
  coordinator retry regression and removal of a redundant legacy-spec edit;
  both were repaired. Round 2 returned PASS with no blocking findings.

## Local verification

- `RUSTC_WRAPPER= cargo check --locked --package surreal-memory-server --no-default-features --features server-only`
  — exit 0; compilation completed in 1 minute 16 seconds.
- `RUSTC_WRAPPER= cargo test --locked --test operation_query_deadline --no-default-features --features server-only`
  — exit 0; 1 test passed. A real isolated SurrealDB server held a large
  receipt query past the 50 ms application deadline, and the same production
  coordinator then committed a subsequent operation.
- `RUSTC_WRAPPER= cargo test --locked --test executor_recovery` — exit 0; 4
  tests passed.
- `RUSTFLAGS='-Dwarnings' cargo build --release --locked --no-default-features --features embedded,metal,local-embeddings`
  — exit 0; release build completed in 37.71 seconds.
- `cargo fmt --all --check` — exit 0 on the merged tree.
- `openspec validate bound-operation-query-deadlines --strict` — exit 0; the
  change is valid on the merged tree.
- `RUSTC_WRAPPER= cargo test --locked --lib operations::tests:: -- --nocapture`
  — exit 0; 17 tests passed, including independent initialization,
  generation-safe overlap, replacement failure, startup retry, and the actual
  coordinator drain retry branch.
- `prometheus-rust-auditor enforce`, `format`, and `inventory` — exit 0; no
  findings. The dependency phase remains a repository baseline limitation:
  `cargo-deny` is not installed, while `cargo audit` reports the same three
  advisories in both the committed and working lockfiles.

## Deployment state

The signed installed binary has SHA-256
`63d4b297a9e4b1ebd2cb61ebac0752a00532eaf611b5b0a967ed9dc875942046`
at both owned install paths, and both copies pass `codesign --verify`.

The deployed server recorded
`operation database reconciliation discovery timed out after 10000ms` instead
of leaving the startup future pending indefinitely. The service stayed ready,
and the durable queue continued to advance after the learning worker loaded.
Backlog recovery is still running, so task 1.3 remains open until the accepted
count reaches zero and the final doctor check exits successfully.
