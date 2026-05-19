# task-stream Spec Delta — Add Steps

## ADDED Requirements

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

### Requirement: Step completion SHALL be idempotent

Each step SHALL carry an `idempotency_key`. Completing a step more than once with
the same key SHALL NOT create duplicate steps or duplicate side effects.

#### Scenario: Replayed completion is a no-op
- **WHEN** `complete_step` is called twice with the same `idempotency_key`
- **THEN** exactly one step is recorded as `completed` and the second call
  returns the already-completed step without re-applying its result

### Requirement: A multi-step process SHALL be resumable

After an interruption, a client SHALL be able to determine the next step to run
from durable storage without re-running completed steps.

#### Scenario: Resume after interruption
- **GIVEN** a stream whose steps 1-2 are `completed` and step 3 is `pending`
- **WHEN** a new session queries the stream
- **THEN** it resumes at step 3 and steps 1-2 are not re-executed
