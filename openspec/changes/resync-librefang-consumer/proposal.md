# Resync the librefang Consumer

## Why

Assessment findings **F-3 (MEDIUM)**, **F-4** and **F-5** (librefang): the
`librefang` repo at `/Users/gqadonis/Projects/references/librefang` consumes
`surreal-memory` and must be resynced to the updated library.

librefang is the lower-risk of the two consumers. `crates/librefang-memory/`
**wraps** `Option<Arc<surreal_memory::SurrealStorage>>` — it does **not**
implement `MemoryStorage`, and it does not call the TaskStream/TaskStep surface.
It uses `RetryConfig::default()`. So the new no-default trait methods and the
new `RetryConfig` field do not break its compile. The expected resync is a
lockfile refresh with no code change — but "expected" must be verified.

This change MUST land after `commit-and-push-library`.

## What Changes

- `cargo update -p surreal-memory` in the librefang workspace to pull the
  pushed commit.
- Run `cargo build` + `cargo test` across the librefang workspace.
- Expected outcome: **no code change**. If the build surfaces drift, fix it —
  and record that the "no-code-change" expectation was wrong.
- Verify migrations v17→v19 apply cleanly against a **populated** librefang
  embedded database on first start.
- Commit the refreshed `Cargo.lock` in the librefang repo.

## Non-goals

- Adopting any new `surreal-memory` API surface in librefang.
- Any change to the `surreal-memory` library.
- Re-pinning to a `rev` (deferred to `reconcile-library-pin-policy`).

## Impact

- Mutates the librefang repo: `Cargo.lock` (and source only if the build
  surfaces unexpected drift).
- No change to surreal-memory-server.
- Depends on `commit-and-push-library`. Independent of `resync-uar-consumer`.
