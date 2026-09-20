## Why

Operation receipts are stored under a deterministic record key derived from `operation_id`, but receipt reads filter the whole table by a duplicate field. On the deployed RocksDB service, a single lookup took 3–10 seconds and caused the durable learning worker to time out after only a few records.

## What Changes

- Read operation receipts and internal operation state by their deterministic record identity.
- Retain the existing `operation_id` field and unique index for contract validation and compatibility.
- Verify submit, replay, conflict, and receipt-read behavior through the operation integration suite.

## Capabilities

### New Capabilities

- `operation-receipt-lookup`: Deterministic receipt retrieval by the durable operation identity.

### Modified Capabilities

None.

## Impact

This changes two read paths in `src/operations.rs`. The HTTP schema, stored record format, migrations, and operation state machine remain unchanged.
