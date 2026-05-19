# Reconcile the Library Pin Policy

## Why

Assessment finding **F-6 (LOW)**: both consumers pin `surreal-memory` by
`branch = "main"`, not by `rev`. surreal-memory-server's `CLAUDE.md` rule #7
describes a `rev` pin — the actual `Cargo.toml` files have drifted to a branch
pin.

A `branch = "main"` pin means any future `cargo update` silently absorbs
**whatever** is on `main`, with no review gate. That is how an unreviewed
library change reaches a consumer's production build unnoticed. The phase
should not leave this unresolved: it should record an explicit, deliberate
decision and reconcile the documentation with reality.

This change lands LAST so it can pin the commit that the resync changes have
*proven* good — verified by `resync-uar-consumer` and `resync-librefang-consumer`
building and testing green against it.

## What Changes

- Record an explicit decision in this change's proposal: keep the
  `branch = "main"` pin (convenient, unreviewed) **or** move both consumers to a
  `rev` pin (reviewable, matches the documented rule).
- If moving to `rev`: update both consumers' `Cargo.toml` to pin the
  **verified** commit from `commit-and-push-library` (verified = changes 2 and 3
  passed against it), and refresh + commit each `Cargo.lock`.
- Reconcile `surreal-memory-server/CLAUDE.md` rule #7 so the documented pin
  policy matches whatever is actually used.

## Non-goals

- Re-running the consumer builds (done in changes 2-3).
- Any library or consumer logic change.

## Impact

- Mutates UAR `Cargo.toml` + `Cargo.lock`, librefang `Cargo.toml` +
  `Cargo.lock`, and `surreal-memory-server/CLAUDE.md`.
- Depends on `resync-uar-consumer` and `resync-librefang-consumer` (it pins the
  commit they verified).
