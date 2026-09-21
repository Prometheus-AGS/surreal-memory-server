## 1. Independent ledger recovery

- [x] 1.1 Open the initial server-mode ledger transport independently while preserving embedded initialization, serialize generation-safe replacement, classify recovered deadline errors, and prove initialization, overlap, replacement failure, stale-only retry, startup retry, and general-storage isolation with focused tests.

## 2. Review and publication

- [x] 2.1 Pass formatting, compilation, focused tests, strict OpenSpec validation, the repository Rust format, enforcement, and inventory audit gates; record dependency and partition baseline findings; pass a fresh isolated critic review; commit, push, and merge a reviewable PR.

## 3. Installed runtime certification

- [ ] 3.1 Build and sign the merged binary, reinstall and restart managed services, drain accepted receipts to zero, and prove service health plus `prometheus doctor --json` success.
