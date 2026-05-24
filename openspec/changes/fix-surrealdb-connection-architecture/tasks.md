# Tasks — fix-surrealdb-connection-architecture

Markers: `[ ]` pending · `[/]` in progress · `[x]` done · `[~]` skipped

## Change 1 — Load-repro harness
- [x] Add `crates/surreal-memory/tests/load_repro.rs` (gated `#[ignore]`)
- [x] Add `crates/surreal-memory/tests/load_repro_baseline.md` (template + run instructions)
- [x] `cargo check --tests -p surreal-memory --features embedded,metal` passes
- [ ] **User runs** the harness against docker-compose + embedded; pastes baseline rows
- [ ] Confirm harness reproduces user-reported symptoms in at least one mode (post-run review)

## Change 2 — ArcSwap connection cell
- [ ] Add `arc-swap = "1"` to `crates/surreal-memory/Cargo.toml`
- [ ] Replace `Arc<std::sync::RwLock<ConnectionState>>` with `Arc<ArcSwap<ConnectionCell>>`
- [ ] Introduce single helper `fn live_db(&self) -> Result<Surreal<Any>>`
- [ ] Migrate 47 call sites to the helper
- [ ] Preserve `ReconnectGuard` cancellation safety
- [ ] Replace `connection_arc()` with typed `ConnectionView` for `PalaceAdapter`
- [ ] Enable `clippy::await_holding_lock` at deny level
- [ ] Verify load harness shows ≥3× p99 improvement (server mode, concurrent hybrid_search)
- [ ] `cargo check --workspace --features embedded,metal` passes
- [ ] `./scripts/quality-check.sh` passes

## Change 3 — Config::query_timeout
- [ ] Add `query_timeout_ms` to `RetryConfig` (default 10_000)
- [ ] Add env var `SURREAL_QUERY_TIMEOUT_MS`
- [ ] Pass `surrealdb::opt::Config::default().query_timeout(...)` at every connect site
- [ ] Update env-var parse tests in `src/main.rs:428-510`
- [ ] Extend comment at `surreal.rs:69-80` documenting timeout layering

## Change 4 — Typed-error retry classification
- [ ] New `crates/surreal-memory/src/storage/retry.rs` with `RetryAction` enum
- [ ] `fn classify(err: &anyhow::Error) -> RetryAction` downcasting to `surrealdb::Error`
- [ ] `retry_operation` uses `classify(...)`
- [ ] Reconnect triggered only on `Reconnect` action
- [ ] Unit tests cover each variant
- [ ] Verify load harness: server-busy errors back off without churning reconnects

## USER CHECKPOINT
- [ ] Surface assessment §6 open questions; pause for answers before Change 5/6

## Change 5 — Workload-isolated sessions (CONDITIONAL)
- [ ] Confirm vertical-scale target (gate)
- [ ] New `crates/surreal-memory/src/storage/sessions.rs` (`SessionPool`)
- [ ] Hot-path helpers `live_read_db()` / `live_write_db()`
- [ ] Env vars `SURREAL_READ_SESSIONS` (4), `SURREAL_WRITE_SESSIONS` (2)
- [ ] Integration test: write-heavy workload no longer increases read p99

## Change 6 — Embedded-mode semaphore + framing
- [ ] `Semaphore` from `SURREAL_EMBEDDED_MAX_INFLIGHT` (default 16)
- [ ] Permit acquire/release in hot path when `SurrealMode::Embedded`
- [ ] Verify load harness: no "serialization failure" / "lock timeout" under bounded concurrency

## Change 7 — Documentation updates
- [x] CLAUDE.md: new "SurrealDB Skill References" section
- [x] CLAUDE.md: append connection-handle + embedded-ceiling rules to "SurrealDB Gotchas"
- [x] AGENTS.md: mirror both additions; sync migration count to current (v16+); mention `palace` feature
- [x] Grep confirms no stale "v8 vs v16" mismatch between the two files
- [x] CLAUDE.md + AGENTS.md: top-of-file "Code-Quality Discipline" with Karpathy 4 + Boris 3 + self-improvement loop
- [x] CLAUDE.md + AGENTS.md: "Rust Skills to Invoke" mapping
- [x] Seed `docs/lessons.md` with the lessons surfaced by this assessment + carried-forward operational rules

## Cross-cutting discipline (apply to every change above)
- [ ] Before each change, invoke the `karpathy-guidelines` skill
- [ ] For Changes 2, 4, 5, 6: invoke `rust-skills:m07-concurrency` and `rust-async-patterns` before writing code
- [ ] For Change 4: also invoke `rust-skills:m06-error-handling`
- [ ] For Change 1: also invoke `rust-skills:m10-performance`
- [ ] Before merging any change: invoke `prometheus-rust-auditor` for a pre-merge review pass
- [ ] After every correction from the user or CI: append a one-line lesson to `docs/lessons.md`
