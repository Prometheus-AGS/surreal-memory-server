## Context

See `proposal.md` for the failure. `OperationService` currently stores a
generation-tagged SDK handle, but it seeds generation zero by cloning the
general `SurrealStorage` handle. Its timeout error is also erased into
`anyhow::Error`, so the coordinator retries every processing failure rather
than the single recovery case the spec permits.

The Surreal SDK handle is clone-safe, but server-mode clones multiplex one
physical WebSocket. An outer Tokio timeout can drop a query future while the
SDK is still unwinding that transport. The server-mode ledger therefore needs
a separately opened SDK connection. Embedded mode is different: opening the
same RocksDB path twice is invalid, and its clone is an in-process handle rather
than a shared remote transport.

## Goals / Non-Goals

**Goals:**

- Make lazy first-use initialization and later replacement use the same
  serialized independent-connection path.
- Preserve the existing public HTTP deadline error text.
- Keep retry classification inspectable after errors acquire context.
- Prove the retry and generation decisions deterministically, with a real
  server integration proving isolation from general storage.

**Non-Goals:**

- Change the Surreal SDK's internal query timeout or general storage reconnect
  policy.
- Retry executor work, payload validation, or arbitrary database failures.
- Add a third connection pool or configurable recovery knobs.

## Decisions

### Lazy independent initialization

`OperationService` starts with no ledger connection. Its async accessor locks
the existing replacement mutex, checks again after acquiring the lock, asks
`SurrealStorage` for an operation-ledger handle, and publishes generation zero.
That storage method opens a fresh authenticated transport in server mode and
clones the live in-process handle in embedded mode. This keeps the synchronous
router builder unchanged while ensuring a server query cannot fall back to the
shared WebSocket.

Making the whole router builder async was rejected because it would broaden
every construction site and still require concurrency control for the first
request. Pre-opening a connection by blocking inside the synchronous builder
was rejected because it can deadlock a Tokio runtime.

### One typed deadline error with recovery state

The deadline error retains the stage, elapsed milliseconds, and whether a
newer ledger generation was available after replacement. Its display remains
the existing API error. The coordinator inspects this type through the error
chain and retries only when recovery succeeded.

String matching was rejected because repository lessons prohibit retry
classification from rendered messages. Retrying every `anyhow::Error` was
rejected because it repeats executor work.

### Generation check under one async mutex

Initialization and replacement share one Tokio mutex. A replacement checks the
currently published generation after locking; if another caller already
advanced it, the replacement succeeds without opening or publishing another
connection. This prevents an older recovery from overwriting a newer handle.

### Evidence split

Pure decision helpers and generation transitions receive focused unit tests,
including connector failure and non-ledger errors. The server integration
holds a large ledger receipt query past the application deadline while a
general storage health query completes, then proves the same production
coordinator accepts and commits later work.

## Risks / Trade-offs

- **[Risk] The first server-mode operation request now pays one connection
  setup.** → The setup happens once and is serialized; embedded mode retains
  its existing in-process cost.
- **[Risk] A failed initial connection leaves the slot empty.** → A later call
  may attempt initialization again; no shared fallback is allowed.
- **[Risk] Real-server timing evidence can be affected by host pressure.** →
  The integration uses an isolated fixture and deterministic large result,
  while retry classification and generation behavior use non-timing tests.

## Migration Plan

1. Merge and build the repaired binary from a clean committed tree.
2. Install and sign both owned binary copies.
3. Restart the database, memory server, and learning worker in dependency order.
4. Observe accepted receipts reach zero and run `prometheus doctor --json`.
5. Roll back to the preceding signed binary if readiness or backlog progress
   regresses; stored receipts require no schema migration.
