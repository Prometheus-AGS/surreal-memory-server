# operation-query-deadlines Specification

## Purpose
Prevent a direct operation-ledger database wait from freezing durable operation
reconciliation while the service remains healthy.

## Requirements

### Requirement: Direct operation-ledger database waits are bounded

The operation service SHALL apply the configured query timeout to every direct
operation-ledger database future. The deadline MUST NOT enclose embedding or
executor protocol work.

#### Scenario: A receipt query does not complete within the configured deadline

- **WHEN** a receipt request reaches the production API router
- **AND** its direct database future does not complete within the configured query timeout
- **THEN** the request returns an error naming the database stage and elapsed deadline
- **AND** the operation coordinator remains able to process later work

#### Scenario: The coordinator performs embedding work

- **WHEN** an operation invokes the supervised embedding executor
- **THEN** the operation database deadline is not active around that executor request
- **AND** executor completion remains governed by the executor watchdog contract
