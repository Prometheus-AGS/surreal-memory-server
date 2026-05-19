# Commit and Push the Library

## Why

Assessment finding **F-1 (BLOCKER)**: both completed library phases —
`task-stream-reliability` and `write-path-reliability`, ~31 uncommitted files
including the entire `task_step.rs` module and migrations v18/v19 — are
uncommitted on surreal-memory-server. `git log` ends at `edca63d`.

Both consumers (UAR, librefang) pin `surreal-memory` by `branch = "main"`.
`cargo update` resolves a branch pin to the latest **pushed** commit; it cannot
see a working tree. Until the two phases are committed and pushed, there is
nothing for the consumer resync to resolve against — the whole
`uar-consumer-resync` phase is blocked on this change.

## What Changes

- The user authorizes the commit (KBD does not commit unsolicited — the first
  task is an explicit go/no-go gate).
- Fix, or explicitly defer with the user's agreement, the pre-existing
  `tests/contract_alignment_test.rs:300/312` compile error (`Option<HashMap>`
  vs `Option<Value>`) so the library's own `cargo check --tests` is green — a
  consumer must not resync to a library whose test build is red.
- Commit the two completed phases with sensible boundaries (ideally one commit
  per archived OpenSpec change, or per KBD phase) and push to `main`.
- Record the resulting commit SHA — the downstream resync changes pin it.

## Non-goals

- Any new library code logic — the two phases' code is already written,
  reviewed, and QA-passed. This change only commits and pushes it.
- The consumer-side resync (changes 2-4).

## Impact

- Mutates surreal-memory-server git history (commits + push to `main`).
- One code edit only: the `contract_alignment_test.rs` type-mismatch fix.
- Unblocks `resync-uar-consumer` and `resync-librefang-consumer`.
