# Lessons

A living list of rules earned by hitting real problems in this repo. Add
one line whenever a mistake is caught — by the user, by review, by CI, by
runtime. Read this file at the start of any non-trivial change.

Format: `YYYY-MM-DD — Rule — *(context: what went wrong)*`

## Connection / storage

- 2026-05-24 — Do not wrap `Surreal<Any>` in `std::sync::RwLock`, `tokio::sync::RwLock`, or `Mutex`. The SDK handle is already `Arc`-wrapped and clone-safe. Clone per task. *(Context: `surreal.rs:44` wrapped the handle in a blocking lock at 47 sites, producing writer starvation, lock poisoning, and timeouts under load.)*
- 2026-05-24 — When SurrealDB-related symptoms are load-dependent and hardware-sensitive, the cause is contention in the application's own concurrency primitives, not in SurrealDB. Look at the lock topology before tuning timeouts. *(Context: the `SURREAL_*_RETRIES` knobs were the wrong layer to fix.)*
- 2026-05-24 — Embedded mode is a first-class production target. Its scaling story is: no application-layer lock + semaphore sized to RocksDB stripe count (16 default) + benchmark to prove it. Do not frame embedded as "dev-only." *(Context: corrected during execute planning — user confirmed embedded is a production deployment shape, not just a dev convenience.)*
- 2026-05-24 — Pass `surrealdb::opt::Config::query_timeout(...)` at connect time. Application-level deadlines are a backstop, not the primary mechanism. *(Context: a slow server-side query head-of-line-blocked the multiplexed WebSocket for unrelated work.)*
- 2026-05-24 — Classify retries on `surrealdb::Error` variants, not on stringified message substrings. *(Context: a "lock timeout" error was being treated as "reconnect needed", which hammered the contended path.)*

## Refactor results

- 2026-05-24 — `ArcSwap<ConnectionCell>` over `Surreal<Any>` lands in 1 file (47 lock sites → 0). Workspace + tests compile clean; `cargo clippy -- -D clippy::await_holding_lock` passes; 41/41 library unit tests pass. The lock topology defect documented above is removed at the source. *(Pending: load-harness re-run to quantify the p99 improvement.)*
- 2026-05-24 — `Config::query_timeout` now flows from `SURREAL_QUERY_TIMEOUT_MS` (default 10s) into every `connect()` site. Application `operation_deadline_ms` is now a backstop, not the primary mechanism.
- 2026-05-24 — `RetryAction { Retry, Reconnect, FailFast }` replaces `is_retriable_error: bool`. `Reconnect` is reserved for transport-level loss; server-busy / lock-timeout / serialization errors backoff-and-retry on the same connection (the previous code churned a reconnect on each, hammering the contended path).
- 2026-05-24 — Embedded-mode in-flight semaphore (`SURREAL_EMBEDDED_MAX_INFLIGHT`, default 16 = RocksDB stripe count) caps concurrent ops at the application layer, turning storage-engine contention into honest backpressure instead of "lock timeout" / "serialization failure" error storms.

## Process / discipline

- 2026-05-24 — Before proposing an architectural fix, verify the claimed root cause with a deterministic load-repro. Without a repro, fixes are unfalsifiable. *(Context: Change 1 of the surrealdb-connection-architecture plan exists to enforce this.)*
- 2026-05-24 — If a doc comment in the file *already* says "no extra wrapping needed" and the code contradicts it, the comment is right and the code is the defect. Read existing doc comments before adding state. *(Context: `surreal.rs:38-42` documented the correct shape; the implementation ignored it.)*
- 2026-05-24 — Tuning a retry/timeout knob is not a fix; it is a band-aid. If you find yourself reaching for a `MAX_*_RETRIES` env var to make a symptom go away, stop and look at the layer below.
- 2026-05-24 — Do not invent skill names. If a skill is referenced in a request but not present in `~/.agents/skills/` or `~/.claude/skills/`, surface the gap; do not pretend it exists.
- 2026-08-26 — A test-failure count is not a fact until it is reproduced. The same suite reported 5 → 1 → 0 → 0 failures on byte-identical code; "fix the 5 failing tests" was an unfalsifiable premise. Re-run before diagnosing, and never report a pass/fail summary from output that was clipped by `tail`.
- 2026-08-26 — `cmd | grep ...` reports **grep's** exit status, not `cmd`'s. `cargo test` failures were masked as exit 0 twice. Capture the real status (`PIPESTATUS`, or read the `test result:` line) before claiming green.
- 2026-08-26 — Never run the integration suite against the shared `:28000` SurrealDB. UAR and the MCP server hold live connections to it, so tests compete with production traffic for the same RocksDB instance — the actual source of the "flaky server-mode tests." Start a scratch server on its own port/datadir and set `TEST_SURREAL_ENDPOINT`. Isolated: 12/12 green in 4-11s; shared: intermittent failures at 47-277s.
- 2026-08-26 — Synchronous FFI initialization (`Device::new_metal`, `new_cuda`) must run on `spawn_blocking`, never directly in an async block. Called inline it blocks a tokio worker, starves the executor child's 250ms heartbeat, and the supervisor's 30s watchdog SIGKILLs the child — 24 kills across 23 generations before this was found. `spawn_blocking` is required for any FFI that can occupy a thread, not just for CPU-bound compute.
- 2026-08-26 — A crate that exposes no timeout configuration still needs one: hf-hub 0.5 builds its reqwest client with no connect or read timeout, so the caller must impose `tokio::time::timeout`. Check the dependency's client construction before assuming a network call is bounded.
- 2026-08-26 — Read the log before accepting a bug report's hypothesis. Issue #5 attributed the kills to a hung download; the timestamps put every one immediately after GPU init instead. The reporter's diagnosis is a hypothesis, not evidence.

## Schema / migrations *(carried forward from CLAUDE.md operational rules)*

- 2026-03-17 — Every field on a persisted Rust struct must have a matching `DEFINE FIELD` migration. SurrealDB SCHEMAFULL rejects unknown fields at runtime.
- 2026-03-17 — `option<object>` is not flexible. Use `FLEXIBLE TYPE option<object>` when callers may pass nested JSON.
- 2026-03-17 — HNSW index dimensions must match embedding dimensions exactly. Memory/entity = 1536d (v5); palace drawers = 384d (v16). Independent vector spaces; never cross-query.

## Agent operational *(carried forward)*

- 2026-03-17 — Read the existing public API (`lib.rs`, `storage/mod.rs`, `migrations/mod.rs`) before writing any new storage layer code. Do not build parallel abstractions.
- 2026-03-17 — When the user says "commit and push", do exactly that. Do not re-verify, re-test, or refactor unrelated code first.
- 2026-03-17 — One `cargo check` per change set. Do not re-run "to be sure."
- 2026-03-17 — Test changes against the real consumer (UAR), not a toy test.
- 2026-03-17 — For CLI/agent consumers, `user_id = "anonymous"` so scope filtering uses `agent_id`.
