# Bound operation-ledger database waits

## Problem

The durable operation coordinator issues database statements directly on the
SurrealDB client. The client connection has a server-query timeout, but these
callers do not have an application deadline while waiting for the shared
connection. In the deployed service, reconciliation stopped making progress
while both `/health` and `/ready` remained healthy.

## Change

- Apply the configured query timeout to every direct operation-ledger database
  future.
- Report the failed database stage and deadline when the bound expires.
- Keep embedding and executor work outside this deadline so timeout
  cancellation cannot abandon an in-flight executor protocol request.

## Non-goals

- Changing the storage retry policy or SDK query timeout.
- Timing out a complete operation or embedding request.

## Capability

- `operation-query-deadlines`: direct operation-ledger database waits are
  bounded on the production API path.
