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

The final deployed source is
`bc3d1ea4d3460afcece646e5655583aee3650744`. The release binary and both owned
installed copies have SHA-256
`981d37e83316e983f383e7de0ff69f945325b1b00968a288ea37cdf708afd290`;
both installed copies pass `codesign --verify`.

The corrected startup discovery uses state-index equality queries with an
independent deadline for each state. The installed service drained 258
accepted durable operations to zero. The final full query over accepted,
validated, blocked, planned, and processing states returned an empty set.
`GET /ready` returned HTTP 200 with all capabilities true, and
`prometheus doctor --json` exited 0 with 15 passed, 0 failed, 3 warned, and 2
skipped checks. The learning worker also reconciled all 261 accepted
filesystem receipts to completed with zero rejected or dead receipts.

The deployed logs include recovered executor and operation-lookup timeouts at
05:37 and 06:18 UTC. They did not prevent the queue from reaching zero and are
retained as measured runtime limitations.
