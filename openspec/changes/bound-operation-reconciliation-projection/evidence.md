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

The projection changes are installed through the signed
`surreal-memory-server` binary whose SHA-256 is
`63d4b297a9e4b1ebd2cb61ebac0752a00532eaf611b5b0a967ed9dc875942046`
at both `~/.local/bin` and `/usr/local/bin`. Both copies pass
`codesign --verify`.

Backlog recovery is still running. The installed service reduced the local
durable queue from 271 accepted and 2,286 completed operations to 261 accepted
and 2,296 completed operations while both database health and memory readiness
returned HTTP 200. Task 1.3 remains open until the accepted count reaches zero
and the final doctor check exits successfully.
