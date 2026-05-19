# Tasks — resync-librefang-consumer

Repo: `/Users/gqadonis/Projects/references/librefang`

## 1. Pull the updated library
- [ ] 1.1 `cargo update -p surreal-memory` to resolve the commit from `commit-and-push-library`
- [ ] 1.2 Confirm `Cargo.lock` now pins the new library commit

## 2. Build, test, verify
- [ ] 2.1 `cargo build` across the librefang workspace
- [ ] 2.2 `cargo test` across the librefang workspace green
- [ ] 2.3 If the build surfaces drift (unexpected), fix it and record that the no-code-change expectation was wrong
- [ ] 2.4 Verify migrations v17→v19 apply cleanly against a populated librefang embedded DB on first start (F-5)

## 3. Commit + QA
- [ ] 3.1 Commit the refreshed `Cargo.lock` (and any source fixes) in the librefang repo
- [ ] 3.2 QA gate: rust-reviewer against `.kbd-orchestrator/constraints.md` (likely a lock-only change — QA may be skipped if <3 files and lock-only, per the kbd-execute skip rule)
