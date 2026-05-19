# write-path Spec Delta — Eliminate Write-Path Stalls

## ADDED Requirements

### Requirement: A write operation SHALL return within a bounded wall-clock budget

Every write operation SHALL either complete or fail with a typed error within a
single configurable wall-clock budget that spans all retry and reconnection
attempts. A write SHALL NOT stall open-endedly.

#### Scenario: Write against an unreachable database is bounded
- **GIVEN** SurrealDB is unreachable
- **WHEN** a client invokes `create_task_stream`
- **THEN** the call returns a typed error within the configured budget
- **AND** the call does not hang for minutes

#### Scenario: Non-embedding write is bounded
- **GIVEN** SurrealDB is unreachable
- **WHEN** a client invokes `create_entity` or `create_task_stream`
- **THEN** both return a bounded typed error, confirming the bound is independent
  of the embedding subsystem

### Requirement: Retry and reconnection SHALL NOT compound unboundedly

Per-operation retry SHALL NOT trigger an unbounded reconnection loop. The total
time spent across retries and reconnections for a single operation SHALL be
capped by the operation budget.

#### Scenario: Reconnect attempts are capped within an operation
- **GIVEN** a retriable failure occurs during a write
- **WHEN** the retry path attempts reconnection
- **THEN** the combined retry + reconnect time stays within the operation budget

#### Scenario: Transient failure within budget still recovers
- **GIVEN** a transient connection failure that resolves quickly
- **WHEN** a write is retried within the budget
- **THEN** the write succeeds and returns the created record
