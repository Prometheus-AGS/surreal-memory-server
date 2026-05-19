# write-path Specification

## Purpose
TBD - created by archiving change bound-embedding-model-loading. Update Purpose after archive.
## Requirements
### Requirement: Embedding model loading SHALL be bounded in time

Loading an embedding model (Candle local provider, or Palace FastEmbed) SHALL
complete or fail with a typed error within a configurable bound. An embedding
provider SHALL NOT block a write request open-endedly on a cold-cache model
download.

#### Scenario: Cold-cache download failure is bounded
- **GIVEN** an empty model cache and an unreachable model source
- **WHEN** the first embedding-dependent write is invoked
- **THEN** the call fails with a clear typed error within the configured bound
- **AND** the failure does not hang for minutes

#### Scenario: Warm cache load succeeds
- **GIVEN** a populated model cache
- **WHEN** the first embedding-dependent write is invoked
- **THEN** the model loads and the write completes successfully

### Requirement: Model loading SHALL NOT block the async runtime

Synchronous, CPU-heavy model load and inference work SHALL run off the tokio
runtime worker threads (via `spawn_blocking` or equivalent) so that one
embedding operation cannot starve unrelated concurrent tasks.

#### Scenario: A non-embedding call is not starved by a model load
- **GIVEN** an embedding model is loading on first use
- **WHEN** a concurrent non-embedding operation is invoked
- **THEN** the non-embedding operation is not blocked by the model load

### Requirement: Server health SHALL reflect true write-path readiness

The `/health` endpoint and startup logs SHALL distinguish "process started"
from "embedding model loaded and verified". The server SHALL NOT report the
embedding service as ready before the model is actually loadable.

#### Scenario: Health distinguishes readiness states
- **WHEN** a client queries `/health` before the embedding model is loaded
- **THEN** the response indicates the embedding subsystem is not yet ready

#### Scenario: Optional warmup pays first-write cost at boot
- **GIVEN** startup warmup is enabled in config
- **WHEN** the server finishes starting
- **THEN** the embedding model is already loaded and the first write incurs no
  cold-load latency

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

### Requirement: Slow tool calls SHALL emit MCP progress notifications

A tool handler whose operation may take longer than a few seconds SHALL emit
`notifications/progress` heartbeats while work is in flight, so a client that
supports `resetTimeoutOnProgress` does not time out on a working server.

#### Scenario: A slow tool reports progress
- **GIVEN** a client supplies a `progressToken` for a slow tool call
- **WHEN** the operation runs longer than the heartbeat interval
- **THEN** the server emits at least one `notifications/progress` before the
  final result

#### Scenario: Missing progress token degrades gracefully
- **GIVEN** a client supplies no `progressToken`
- **WHEN** a slow tool is invoked
- **THEN** the operation still completes correctly without emitting progress

### Requirement: Mindmap updates SHALL enforce a bounded query timeout

The mindmap `UPDATE` path SHALL enforce a query `TIMEOUT` so that updates to
large mindmaps fail fast with a clear error rather than hanging. The implemented
behavior SHALL match the documentation.

#### Scenario: Large mindmap update fails fast
- **GIVEN** a mindmap large enough that an update exceeds the timeout
- **WHEN** a node or edge is added
- **THEN** the operation returns a clear error within the timeout bound

#### Scenario: Documentation matches implementation
- **WHEN** the mindmap timeout behavior is reviewed against CLAUDE.md
- **THEN** the documented timeout matches the enforced timeout in code

