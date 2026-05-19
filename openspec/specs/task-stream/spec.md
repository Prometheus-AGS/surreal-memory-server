# task-stream

## Purpose

The TaskStream capability provides named, long-running task context that
accumulates scoped memories for multi-step processes, with per-tenant scope
isolation and safe rolling auto-summarization.

## Requirements

### Requirement: Stream resolution SHALL be scope-bounded

`get_task_stream` and all operations that resolve a stream by name SHALL accept
the caller's `agent_id` and `user_id` and SHALL only return or mutate a stream
whose `agent_id`/`user_id` match the caller's scope.

#### Scenario: Cross-scope read is rejected
- **WHEN** agent A requests a stream named `build` that is owned by agent B
- **THEN** the result is `None` (not B's stream)

#### Scenario: Cross-scope mutation is rejected
- **WHEN** agent A calls `add_to_task_stream`/`archive_task_stream`/`pause_task_stream`/`delete_task_stream`
  for a stream owned by agent B
- **THEN** the operation fails with a not-found error and B's stream is unchanged

### Requirement: Auto-summarization SHALL only compact its own stream

`auto_summarize_task_stream` SHALL select and delete only memories whose
`task_stream_id` equals the target stream's id.

#### Scenario: Sibling stream is untouched
- **GIVEN** an agent owns two active streams, S1 and S2, each over the
  summarization threshold
- **WHEN** `auto_summarize_task_stream` runs for S1
- **THEN** only S1's memories are compacted and S2's memories are unchanged

### Requirement: Token accounting SHALL be atomic and accurate

Adding a memory to a stream SHALL update `total_tokens` in the same transaction
as the memory insert. Summarization SHALL subtract the compacted memories' token
totals from `total_tokens` and add back the generated summary's token count.

#### Scenario: Concurrent adds do not lose token counts
- **WHEN** N memories are added concurrently to one stream
- **THEN** the final `total_tokens` equals the sum of all N per-memory counts

#### Scenario: Summarization decrements the token total
- **WHEN** `auto_summarize_task_stream` compacts memories
- **THEN** `total_tokens` decreases by the compacted memories' token total
  (less the summary's own tokens) and the summarization trigger does not
  immediately re-fire

### Requirement: Stream names SHALL be unique per scope, not globally

The uniqueness constraint on a stream name SHALL be scoped to
`(agent_id, user_id, name)`.

#### Scenario: Two agents reuse a stream name
- **WHEN** agent A and agent B each create a stream named `build`
- **THEN** both `create_task_stream` calls succeed

### Requirement: Summary memories SHALL remain attached to their stream

A memory produced by auto-summarization SHALL carry the `task_stream_id` of the
stream it summarizes.

#### Scenario: Summary is included in task context
- **WHEN** a summary memory is created for stream S
- **THEN** `get_context_for_task(S)` includes that summary memory

### Requirement: A TaskStream SHALL support ordered, status-tracked steps

A `TaskStream` SHALL be able to contain `TaskStep` records, each with an
`ordinal`, a `name`, and a `status` of `pending`, `running`, `completed`,
`failed`, or `skipped`.

#### Scenario: Steps are created and ordered
- **WHEN** a client adds steps 1, 2, 3 to a stream
- **THEN** `get_task_steps` returns them in ordinal order with status `pending`

#### Scenario: Current step is reported
- **GIVEN** a stream where steps 1-2 are `completed` and step 3 is `pending`
- **WHEN** the client calls `get_current_step`
- **THEN** step 3 is returned

#### Scenario: A failed step blocks progress until resolved
- **GIVEN** a stream where step 1 is `failed` and step 2 is `pending`
- **WHEN** the client calls `get_current_step`
- **THEN** the failed step 1 is returned so the caller can retry, skip, or
  complete it before advancing

### Requirement: Step completion SHALL be idempotent

Each step SHALL carry an `idempotency_key`. Completing a step more than once with
the same key SHALL NOT create duplicate steps or duplicate side effects.

#### Scenario: Replayed completion is a no-op
- **WHEN** `complete_step` is called twice with the same `idempotency_key`
- **THEN** exactly one step is recorded as `completed` and the second call
  returns the already-completed step without re-applying its result

#### Scenario: Concurrent duplicate add is idempotent
- **WHEN** `add_task_step` is called concurrently twice with the same
  `idempotency_key`
- **THEN** exactly one step is created and both calls return the same step

### Requirement: A multi-step process SHALL be resumable

After an interruption, a client SHALL be able to determine the next step to run
from durable storage without re-running completed steps.

#### Scenario: Resume after interruption
- **GIVEN** a stream whose steps 1-2 are `completed` and step 3 is `pending`
- **WHEN** a new session queries the stream
- **THEN** it resumes at step 3 and steps 1-2 are not re-executed
