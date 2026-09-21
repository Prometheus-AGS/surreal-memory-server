## Purpose

Keep durable operation-ledger cancellation and recovery isolated from ordinary
memory storage while preserving bounded, generation-safe retry behavior.

## ADDED Requirements

### Requirement: The server-backed operation ledger owns an independent transport

Before executing its first ledger query, the operation service SHALL open a
server-mode ledger connection independently of the general storage transport.
Concurrent first use MUST converge on one published ledger generation. In
embedded mode it SHALL instead clone the in-process SDK handle because a second
RocksDB open on the same path is invalid; the clone MUST NOT publish or mutate
general storage connection state.

#### Scenario: First ledger query overlaps ordinary storage work

- **WHEN** the first operation-ledger query is still running
- **THEN** ordinary server-mode storage work remains able to complete on its own transport
- **AND** cancellation of the ledger query does not change general storage connection state

#### Scenario: Concurrent callers initialize the ledger

- **WHEN** multiple operation requests arrive before a ledger connection has been published
- **THEN** initialization is serialized
- **AND** every caller observes the same published ledger generation

#### Scenario: Embedded ledger initializes without reopening RocksDB

- **WHEN** the operation service uses an embedded database
- **THEN** ledger initialization clones the existing in-process SDK handle
- **AND** it does not attempt a second open of the embedded database path

### Requirement: Ledger replacement is generation safe

The operation service SHALL serialize ledger replacement and SHALL replace a
connection only while the caller's generation is still current. A replacement
started for an older generation MUST NOT overwrite a newer published
connection.

#### Scenario: Timed-out queries overlap

- **WHEN** multiple queries from the same ledger generation time out concurrently
- **THEN** at most one replacement connection becomes the next generation
- **AND** every later replacement attempt observes that newer generation

#### Scenario: Replacement cannot connect

- **WHEN** a timed-out ledger query cannot establish a replacement connection within its bound
- **THEN** the request returns the database deadline error
- **AND** the failure is not classified as a recovered stale-ledger interruption
- **AND** non-database operation work is not retried

### Requirement: Retry is limited to recovered stale-ledger interruption

The operation coordinator SHALL retry work at most once only when a database
deadline interrupted a stale ledger generation and a replacement generation
was successfully published. Executor, validation, payload, and ordinary
database errors MUST NOT enter this retry path.

#### Scenario: Coordinator work is interrupted by a stale ledger generation

- **WHEN** a coordinator operation returns the typed recovered stale-ledger condition
- **THEN** the same operation is attempted once on the replacement generation
- **AND** a second failure is recorded without another retry

#### Scenario: Executor work fails

- **WHEN** a coordinator operation fails in the executor or another non-ledger stage
- **THEN** the failure is recorded
- **AND** the coordinator does not retry it through ledger recovery

#### Scenario: Startup discovery is interrupted by a stale ledger generation

- **WHEN** startup discovery returns the typed recovered stale-ledger condition
- **THEN** discovery is attempted once on the replacement generation
- **AND** any other startup error is reported without this retry

### Requirement: Backlog discovery and dependency rescans remain bounded

The operation coordinator SHALL discover nonterminal work through the indexed
state values `accepted`, `validated`, `blocked`, and `processing`. Each state
query SHALL have its own database deadline, and the combined identities SHALL
be deduplicated and sorted before processing. After processing a drain wave,
the coordinator SHALL rescan at most once when that wave committed work, so
newly unblocked dependencies are recovered without one full-ledger query per
commit.

#### Scenario: A large terminal history shares the ledger

- **WHEN** startup reconciliation runs beside many committed or rejected rows
- **THEN** discovery queries only the four indexed nonterminal state values
- **AND** every discovered operation identity is processed in deterministic order

#### Scenario: A dependency commits during a drain wave

- **WHEN** an operation was blocked earlier in the wave and its dependency later commits
- **THEN** the coordinator performs one nonterminal rescan after the wave
- **AND** the newly unblocked operation is processed in the next wave
- **AND** the coordinator does not rescan the full ledger after every individual commit
