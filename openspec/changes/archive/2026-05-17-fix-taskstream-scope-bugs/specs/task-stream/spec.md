# task-stream Spec Delta — Fix Scope Bugs

## MODIFIED Requirements

### Requirement: Stream resolution SHALL be scope-bounded

`get_task_stream` and all operations that resolve a stream by name SHALL accept
the caller's `agent_id` and `user_id` and SHALL only return or mutate a stream
whose `agent_id`/`user_id` match the caller's scope.

#### Scenario: Cross-scope read is rejected
- **WHEN** agent A requests a stream named `build` that is owned by agent B
- **THEN** the result is `None` (not B's stream)

#### Scenario: Cross-scope mutation is rejected
- **WHEN** agent A calls `archive_task_stream`/`pause_task_stream`/`delete_task_stream`
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
