# Tasks — commit-and-push-library

## 1. Authorization gate
- [x] 1.1 User authorized committing and pushing the two completed phases to `surreal-memory-server` `main`

## 2. Pre-commit library health
- [x] 2.1 Fixed the pre-existing `contract_alignment_test.rs:300` compile error — aligned `AddMindmapNodeParams.metadata` to `Option<serde_json::Value>` to match the canonical `MindMapNode.metadata` and the REST `AddMindmapNodeRequest.metadata`; dropped the now-unneeded HashMap→Value conversion. Also fixed 8 pre-existing clippy `-D warnings` (no-op `.into()`, `assert!(true)`, `len() >= 1`, useless borrow) in the two test files.
- [x] 2.2 `cargo check --workspace --features embedded,metal --tests` green
- [x] 2.3 `cargo fmt --all --check` clean; `cargo clippy --workspace --features embedded,metal --all-targets -- -D warnings` clean; 110 tests pass

## 3. Commit and push
- [x] 3.1 Two commits: `f065902` (the two library phases + the contract test fix + openspec specs/changes), `9051c3d` (KBD orchestrator artifacts). Local tooling dirs (`.claude/`, `.codex/`, `.opencode/`) deliberately excluded.
- [x] 3.2 Pushed to `main` — `edca63d..9051c3d`
- [x] 3.3 Resync target SHA recorded in `progress.json`: `9051c3d37bfa0bb081b7203b5cb84ae69674cfda`

## 4. Verification
- [x] 4.1 `git rev-parse origin/main` == `9051c3d` == local HEAD; remote `main` updated
- [x] 4.2 QA gate: rust-reviewer on the `AddMindmapNodeParams.metadata` contract-alignment fix
