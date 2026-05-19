# Tasks — commit-and-push-library

## 1. Authorization gate
- [ ] 1.1 Confirm with the user that the two completed phases may be committed and pushed to `surreal-memory-server` `main`

## 2. Pre-commit library health
- [ ] 2.1 Fix the pre-existing `tests/contract_alignment_test.rs:300/312` compile error (`Option<HashMap>` vs `Option<Value>`), or defer it with explicit user agreement
- [ ] 2.2 `cargo check --workspace --features embedded,metal --tests` is green
- [ ] 2.3 `cargo fmt --all --check` + `cargo clippy --workspace --features embedded,metal -- -D warnings` clean

## 3. Commit and push
- [ ] 3.1 Stage and commit the two phases with sensible boundaries (per archived OpenSpec change or per KBD phase)
- [ ] 3.2 Push to `main`
- [ ] 3.3 Record the resulting commit SHA in `progress.json` for the downstream resync changes

## 4. Verification
- [ ] 4.1 `git log` on `main` shows the pushed commits; remote `main` matches local
- [ ] 4.2 QA gate: rust-reviewer on the `contract_alignment_test.rs` fix (the only code edit)
