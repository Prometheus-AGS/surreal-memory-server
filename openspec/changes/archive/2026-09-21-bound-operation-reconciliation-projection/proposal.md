## Why

Startup reconciliation selects every field from every nonterminal operation,
including the full memory payload. The deployed server stalled before resuming
305 accepted operations while the same records remained individually readable.
The coordinator needs only each operation identity to schedule recovery.

## What Changes

- Project only `operation_id` when listing nonterminal work.
- Project only public receipt fields when clients poll an operation.
- Retain deterministic ordering and the existing state filter.
- Add database-backed regressions proving large payload bytes do not enter
  reconciliation or receipt query results.

## Capabilities

### New Capabilities

- `operation-reconciliation-projection`: Bounds startup work discovery to the
  identity data required by the coordinator.

### Modified Capabilities

None.

## Impact

This changes two internal queries and their row types in `src/operations.rs`. The
operation schema, state machine, processing order, HTTP contract, and stored
payloads remain unchanged.
