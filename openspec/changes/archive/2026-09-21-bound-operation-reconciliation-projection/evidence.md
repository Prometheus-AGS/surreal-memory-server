# Verification evidence

Date: 2026-09-21

## Source and review

- PR #18: https://github.com/Prometheus-AGS/surreal-memory-server/pull/18
  - source: `16db67a94230f93a73be6a608cb4b357ec0231c4`
  - merge: `199651d8ec632a1918f64d3e9f015b5a18f84111`
- PR #19: https://github.com/Prometheus-AGS/surreal-memory-server/pull/19
  - source: `9e9c180d4f9bc8c88b96770ea413e9a1c2bcc233`
  - merge: `c88144158d8e7bfc6f690d88586e5e68183521d7`
- The PR #19 isolated artifact critic returned PASS. PR #18 also passed the
  repository Rust audit gate with no blocking finding.

## Local verification

- `RUSTC_WRAPPER= cargo test --locked operations::tests::startup_reconciliation -- --nocapture`
  — exit 0; 2 tests passed.
- `RUSTC_WRAPPER= cargo test --locked operations::tests::startup_reconciliation_query_projects_only_operation_identity -- --nocapture`
  — exit 0; 1 test passed.
- `cargo fmt --all --check` — exit 0 on the merged tree.
- `openspec validate bound-operation-reconciliation-projection --strict` —
  exit 0; the change is valid on the merged tree.

## Deployment state

The final deployed source is
`bc3d1ea4d3460afcece646e5655583aee3650744`. The signed release binary and both
owned installed copies have SHA-256
`981d37e83316e983f383e7de0ff69f945325b1b00968a288ea37cdf708afd290`;
both installed copies pass `codesign --verify`.

The installed service drained 258 accepted durable operations to zero. The
final full query over accepted, validated, blocked, planned, and processing
states returned no rows. The learning worker reconciled 261 accepted
filesystem receipts to completed, leaving zero accepted, rejected, or dead
files. `GET /ready` returned HTTP 200 with every reported capability true, and
`prometheus doctor --json` exited 0 with 15 passed and 0 failed checks.

The deployed logs contain recovered operation lookup and executor persistence
timeouts after startup. They did not prevent complete recovery and remain
recorded limitations.
