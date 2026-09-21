## Why

The first operation-ledger query still uses the shared storage connection, so
cancelling that query can poison unrelated memory persistence before rotation
begins. The current recovery retry also catches executor failures, which can
repeat non-database work and violates the intended retry boundary.

## What Changes

- Open the server-mode operation ledger on an independent connection before
  its first query, including startup reconciliation and concurrent API
  requests; retain one clone-safe in-process handle for embedded mode, where a
  second RocksDB open on the same path is invalid.
- Serialize connection initialization and replacement so an older recovery
  cannot overwrite a newer healthy generation.
- Classify a database deadline as retryable only after the stale ledger
  generation has been replaced.
- Retry startup discovery and an interrupted coordinator operation once only
  for that typed stale-ledger condition.
- Add deterministic regressions for initialization, overlapping replacement,
  replacement failure, retry classification, and startup recovery.

## Capabilities

### New Capabilities

- `operation-ledger-connection-recovery`: Independent connection ownership,
  generation-safe replacement, and narrowly classified retry behavior for the
  durable operation ledger.

### Modified Capabilities

None.

## Impact

This changes `src/operations.rs`, the `SurrealStorage` independent-connection
API, focused operation tests, and the installed server runtime. The
uncomfortable constraint is that the deployed backlog cannot be certified
until the repaired binary is installed and processes every accepted receipt;
passing isolated tests alone does not close the release task.
