# Tasks

Verification follows the repository's phase-gated rule: implement a group,
then run its integration target once. Server-mode tests need a scratch
SurrealDB 4.0 at `TEST_SURREAL_ENDPOINT` (default `ws://127.0.0.1:28100`).
Never point them at the shared `:28000` instance.

## 1. Confirm the query shape on SurrealDB 4.0

- [x] 1.1 In a new integration test file `crates/surreal-memory/tests/memory_enumeration.rs`, add an embedded-mode test that seeds synthetic memories with embeddings and proves `SELECT * OMIT embedding ... AND id > $after ORDER BY id ASC LIMIT $n` returns rows without embeddings in ascending ID order; verify the test passes
- [x] 1.2 Add the `#[ignore]` server-mode variant of 1.1 using the `TEST_SURREAL_ENDPOINT` convention from `tests/expiry.rs`; verify it passes against a scratch server with `--ignored`
- [x] 1.3 Run `EXPLAIN` on the keyset query with and without a `user_id` filter in both modes and record the plans in `crates/surreal-memory/tests/memory_enumeration_baseline.md`; if the filtered query scans and sorts the whole table, add the composite index as a new migration and re-record the plan

## 2. Paginated listing in the library

- [x] 2.1 Add `MemoryPageRequest`, `MemoryPage` and the four limit constants (default 50, max 200, max with embeddings 25, byte budget 1 MiB) in the library and re-export them from `lib.rs`; verify `cargo check -p surreal-memory` passes
- [x] 2.2 Add `MemoryStorage::list_memories_page` with a default body that returns a "pagination is not supported" error; verify with a test that a minimal trait implementation relying on the default returns that error
- [x] 2.3 Implement `list_memories_page` in `SurrealStorage`: validate limit and cursor, apply owner filters and the live view with `$now` bound once, keyset on record ID, fetch `limit + 1`, apply the byte budget, set `next_cursor` and `has_more`; verify with tests for default size, limit zero, limit above each maximum, malformed cursor, empty scope and last page
- [x] 2.4 Add enumeration tests on a multi-page synthetic scope: collected IDs equal stored IDs with no duplicates, resume from a saved cursor, concurrent insert between pages, concurrent delete between pages, expired and archived rows hidden; verify they pass in embedded mode
- [x] 2.5 Add projection tests: no embeddings by default, embeddings present when requested; verify they pass
- [x] 2.6 Add byte-budget tests: large content shortens the page with `has_more` set, and one memory larger than the budget is returned alone and the cursor moves past it; verify they pass
- [x] 2.7 Add scope tests: two users, a caller filtered to one never receives the other's memories, including when given a cursor taken from the other user's listing; verify they pass
- [x] 2.8 Attach request context (operation, page size, embedding flag, which filters were set) to query errors and log it at `warn`; verify with a test that forces a query failure and asserts the error text has the context and contains no seeded content or scope value
- [x] 2.9 Add `#[ignore]` server-mode variants of the 2.3 to 2.7 tests; verify the whole file passes against a scratch server with `--ignored`

## 3. Legacy complete listing and bulk delete

- [x] 3.1 Reimplement `SurrealStorage::get_all_memories` as a loop over internal pages that returns the complete set with embeddings or an error, and remove the unbounded `list_memories_scoped` once it has no callers; update the trait doc comment to mark it legacy and point at `list_memories_page`; verify the existing `integration_test`, `search_correctness`, `expiry` and `team_scoping` suites still pass unchanged
- [x] 3.2 Add a test that the complete listing over a multi-page scope returns every memory once, and that a failing page yields an error rather than a partial list; verify it passes
- [x] 3.3 Reimplement `delete_all_memories` to select IDs only in ID-ordered batches on the physical view and delete through `delete_memory`; verify with tests that a scope larger than one batch is emptied with the right count, another user's memories remain, and expired memories are deleted and counted
- [x] 3.4 Add a `FaultPoint` for bulk deletion and wrap failures with the count deleted so far; verify with a test that the error states the count and that a rerun deletes the remainder

## 4. REST and MCP transports

- [x] 4.1 Change REST `GET /api/v1/memory` to accept `limit`, `cursor` and `include_embeddings` and return `MemoryPage`, keeping the principal-derived owner; verify with HTTP tests for the page object shape, following cursors to completion, a 400 for an over-maximum limit and for a malformed cursor, and a restricted principal never receiving another user's memories
- [x] 4.2 Change MCP `get_all_memories` to take a dedicated params struct with `limit`, `cursor` and `include_embeddings` using `crate::coerce::opt_number` and `crate::coerce::opt_bool`, and return `MemoryPage`; verify with a tool-call test that string-encoded `"25"` and `"false"` are accepted and that the result is the page object
- [x] 4.3 Extract one shared persona collector that pages without embeddings and keeps five truncated items and a total per category, and use it from both `src/mcp/handlers.rs` and `src/api/mindmaps.rs`; verify with a test over a multi-page, multi-category user that the mindmap has one branch per category and its description states the true total
- [x] 4.4 Update the `get_all_memories` and `delete_all_memories` tool descriptions and the `instructions` text in `src/mcp/mod.rs` to describe paging; verify the tool list test or schema output shows the new parameters
- [x] 4.5 Update `website/docs/reference/rest-api.md`, `website/docs/reference/mcp-tools.md` and `docs/MCP_REST_CONTRACT_MATRIX.md` with the page object, the parameters, the limits, the consistency rules and the breaking-change note; verify each documented request matches a request made in the 4.1 and 4.2 tests

## 5. Measurement and integration

- [x] 5.1 Add an `#[ignore]` measurement test that seeds a synthetic scope whose total encoded size is many times the byte budget, enumerates it in both modes, and prints the largest page in bytes with and without embeddings; record the numbers in `memory_enumeration_baseline.md` and verify every page is within the budget
- [x] 5.2 On the scratch server only, attempt the pre-change unbounded query against the same synthetic scope and record in the baseline file whether it failed, with what error and at what encoded size, stating plainly if the reset was not reproduced
- [ ] 5.3 Run the quality gate once: `cargo fmt --all --check`, then `cargo clippy --all-targets --features embedded,metal -- -D warnings`, then `cargo test --all-targets --features embedded,metal`, serially; verify all three pass and record the `test result:` lines
- [x] 5.4 Append any rule this work exposed to `docs/lessons.md`; verify the file has the new dated lines or state that none were needed

## 6. Review findings

- [x] 6.1 Read a page in two queries (IDs and stored sizes first, then the accepted ID range) so the database response is bounded by the byte budget plus one memory; verify with a server-mode test that `limit=200` over 200 memories of 400 KiB succeeds with shortened pages and enumerates every memory once
- [x] 6.2 Fail a page whose next cursor equals its request cursor; verify with a test
- [x] 6.3 Truncate persona leaf labels on a character boundary; verify with a test whose content has a multi-byte character across byte 80
- [x] 6.4 Emit the MCP page as compact JSON so it stays within the byte budget; verify the MCP test asserts the text length against the budget plus envelope
- [x] 6.5 Correct the docs: bulk delete reruns finish only once a failing row is repaired, and a syntactically valid cursor is accepted and constrained by scope; verify the docs match the code
- [x] 6.6 Add an MCP test that a malformed cursor is an invalid-params error; verify it passes

## Workflow follow-up

- Run an independent review of the diff before the PR (`prometheus-rust-auditor` read-only checks and the `memory-verifier` role).
- Open a PR that closes issue #62 and calls out the two breaking transport responses.
- After merge, resync UAR through the consumer-sync process and forward `list_memories_page` in its `MemoryStorage` implementation.
- Archive this change once the PR is merged.
