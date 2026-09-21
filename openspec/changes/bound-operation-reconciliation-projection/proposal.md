## Why

Startup reconciliation selects every field from every nonterminal operation,
including the full memory payload. The deployed server stalled before resuming
305 accepted operations while the same records remained individually readable.
The coordinator needs only each operation identity to schedule recovery.

## What Changes

- Project only `operation_id` when listing nonterminal work.
- Retain deterministic ordering and the existing state filter.
- Add a database-backed regression proving large payload bytes do not enter the
  reconciliation list result.

## Capabilities

### New Capabilities

- `operation-reconciliation-projection`: Bounds startup work discovery to the
  identity data required by the coordinator.

### Modified Capabilities

None.

## Impact

This changes one internal query and its row type in `src/operations.rs`. The
operation schema, state machine, processing order, HTTP contract, and stored
payloads remain unchanged.
