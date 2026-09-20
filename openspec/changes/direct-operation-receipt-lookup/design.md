## Context

Operation creation writes `memory_operation:<sha256(operation_id)>`, while `get` and `get_db` issue `SELECT ... WHERE operation_id = $id LIMIT 1`. The uncomfortable fact is that the unique secondary index did not keep deployed receipt reads within the worker's ten-second request budget under heavy memory pressure; one lookup took 10.64 seconds before restart and 3.21 seconds after warmup.

## Goals

- Make receipt retrieval address the record that submission already created.
- Preserve receipt conversion, missing-record behavior, replay semantics, and conflicts.
- Avoid schema or migration changes.

## Decision

Use the Surreal client record selection API with table `memory_operation` and the existing `record_key(operation_id)` value in both `get` and `get_db`. The record key is SHA-256 hex, so it is deterministic and safe as a Surreal record identifier. The stored `operation_id` and its unique index remain available for validation and older queries.

## Risks

Rows created outside the operation service under a noncanonical record key will no longer be discoverable through the API. Such a row violates the service's existing creation contract; migrations and the operation submit path already use canonical keys.

## Verification

- Run the operations unit and integration tests, including duplicate submission, conflict, restart, and receipt lookup cases.
- Measure the deployed GET endpoint before and after installing the rebuilt server.
- Run the learning worker against the durable backlog and record delivered counts and timeouts.
