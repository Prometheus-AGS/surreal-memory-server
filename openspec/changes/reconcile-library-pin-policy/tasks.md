# Tasks — reconcile-library-pin-policy

## 1. Decision
- [ ] 1.1 Decide: keep `branch = "main"` pin vs move both consumers to a `rev` pin; record the decision and rationale in proposal.md before archive

## 2. Apply the decision
- [ ] 2.1 If `rev`: update UAR `Cargo.toml` to pin the verified commit from `commit-and-push-library`; `cargo update -p surreal-memory`; commit `Cargo.toml` + `Cargo.lock` in the UAR repo
- [ ] 2.2 If `rev`: update librefang `Cargo.toml` to pin the same verified commit; `cargo update -p surreal-memory`; commit `Cargo.toml` + `Cargo.lock` in the librefang repo
- [ ] 2.3 If keeping `branch`: no Cargo.toml change; document the accepted risk

## 3. Reconcile documentation
- [ ] 3.1 Update `surreal-memory-server/CLAUDE.md` rule #7 so the documented pin policy matches what is actually used

## 4. Verification
- [ ] 4.1 If `rev`: both consumers still `cargo build` green on the pinned commit
- [ ] 4.2 QA gate: rust-reviewer (small — Cargo.toml + doc)
