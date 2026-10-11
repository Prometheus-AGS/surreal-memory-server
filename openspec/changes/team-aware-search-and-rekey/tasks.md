Records work that already merged in #44. The evidence is its integration tests.

- [x] 1. Lean search DTO with an `include_embeddings` opt-in (REST and MCP).
- [x] 2. Categories in KNN and BM25, with the storage trait signature and all callers updated.
- [x] 3. `rekey_agent_id` operation, connect-info wiring and loopback guard. OpenAPI enum updated.
- [x] 4. `tests/team_scoping.rs`: 4/4, in-process over the embedded engine. `crates/surreal-memory/tests/search_correctness.rs`: 2/2.
- [x] 5. Bump the version to 1.10.0 (Cargo.toml, OpenAPI document).
- [ ] 6. Tag `v1.10.0` on the merge commit, as authorized by the operator.
