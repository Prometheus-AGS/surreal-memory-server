# task-stream Spec Delta — Harden Correctness

## MODIFIED Requirements

### Requirement: Token accounting SHALL be atomic and accurate

Adding a memory to a stream SHALL update `total_tokens` in the same transaction
as the memory insert. Summarization SHALL subtract the compacted memories' token
totals from `total_tokens`.

#### Scenario: Concurrent adds do not lose token counts
- **WHEN** N memories are added concurrently to one stream
- **THEN** the final `total_tokens` equals the sum of all N per-memory counts

#### Scenario: Summarization decrements the token total
- **WHEN** `auto_summarize_task_stream` compacts memories
- **THEN** `total_tokens` decreases by the compacted memories' token total and
  the summarization trigger does not immediately re-fire

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
