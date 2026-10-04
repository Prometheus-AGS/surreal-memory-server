## ADDED Requirements

### Requirement: Search responses omit embeddings by default
REST and MCP search responses SHALL NOT include `embedding` unless the request sets `include_embeddings: true`.

#### Scenario: Lean by default
- **WHEN** `POST /api/v1/search` is called without `include_embeddings`
- **THEN** no result has an `embedding` key, and results are non-empty for a seeded match

### Requirement: Categories filter inside nearest-neighbour search
When `categories` is non-empty, search SHALL return only memories carrying at least one listed category. It SHALL return up to `limit` such rows, even when none of them are among the unfiltered top results.

#### Scenario: Filtered rows outside the unfiltered top-k
- **WHEN** three tagged memories lie outside the unfiltered top-3 and `categories` names their tag with `limit` 3
- **THEN** exactly those three are returned, on both the REST hybrid path and the KNN path

### Requirement: Unattributed records can be re-keyed from the host
The `rekey_agent_id` operation SHALL re-key `memory` and `task_stream` records whose `agent_id` is NONE, NULL or `""`. It SHALL write nothing with `dry_run`. It SHALL be idempotent per `operation_id`. It SHALL be refused with 403 unless the peer is loopback.

#### Scenario: Dry run, apply, replay
- **WHEN** a dry run, then an apply, then a replay of the apply are submitted from loopback
- **THEN** the dry run reports the counts and writes nothing, the apply re-keys only unattributed records, and the replay returns 200 without running again

#### Scenario: Remote peer refused
- **WHEN** the operation is submitted from a non-loopback peer, or without connect-info
- **THEN** the response is 403
