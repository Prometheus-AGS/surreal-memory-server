## Purpose

Ensure durable operation receipts can be retrieved by their stable identity without work proportional to unrelated ledger rows.

## ADDED Requirements

### Requirement: Receipt lookup uses durable operation identity
The ledger SHALL retrieve an operation by the deterministic record identity derived from the requested `operation_id`, and SHALL return only a record whose durable identity corresponds to that request.

#### Scenario: Existing operation is requested
- **WHEN** a caller requests a receipt for an `operation_id` that was durably accepted
- **THEN** the ledger returns that operation's current receipt without filtering unrelated operation rows

#### Scenario: Unknown operation is requested
- **WHEN** a caller requests an `operation_id` whose deterministic record does not exist
- **THEN** the ledger returns the existing not-found response

### Requirement: Submit replay uses the same identity lookup
The ledger SHALL use the same deterministic receipt lookup after submission and when resolving a concurrent or replayed operation ID.

#### Scenario: Existing ID is submitted again
- **WHEN** a caller submits an existing `operation_id` with the same payload hash
- **THEN** the ledger returns the authoritative existing receipt without creating a second operation

#### Scenario: Existing ID has another payload hash
- **WHEN** a caller submits an existing `operation_id` with a different payload hash
- **THEN** the ledger returns the existing conflict response
