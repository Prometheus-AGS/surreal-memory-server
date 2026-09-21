## Purpose

Bound durable operation recovery discovery independently of stored payload size.

## ADDED Requirements

### Requirement: Reconciliation discovery projects only operation identity

The durable coordinator SHALL list nonterminal operations by projecting only
the stable operation identity required to schedule processing. It MUST preserve
deterministic operation-ID ordering and MUST NOT read stored payload, result, or
executor fields into the reconciliation list result.

#### Scenario: Many nonterminal operations contain large payloads
- **WHEN** startup reconciliation discovers accepted, blocked, or processing operations
- **THEN** its list result contains only each `operation_id` in deterministic order
- **AND** payload size does not increase the returned reconciliation data

#### Scenario: Terminal operations share the ledger
- **WHEN** committed or rejected operations exist beside nonterminal operations
- **THEN** the reconciliation list excludes terminal identities without loading their payloads

### Requirement: Receipt lookup excludes request payloads

The durable coordinator SHALL return an operation receipt by projecting only
the fields in the public receipt contract. It MUST NOT load the stored request
payload into the receipt query result.

#### Scenario: A client polls an operation with a large request payload
- **WHEN** the client retrieves that operation's receipt by its stable identity
- **THEN** the query result contains every public receipt field
- **AND** the query result does not contain the stored request payload
