# Proposal: Bound startup embedding-dimension validation

## Problem

Server startup selects every full embedding vector from the `entity` and
`memory` tables to verify dimensions. The deployed database returned roughly
50 MB for this validation and held the REST API before bind under swap pressure.

## Change

Project each vector's length inside SurrealDB and transfer only the record ID
and integer dimension to the server. Preserve the existing mismatch error and
the validation of both embedding tables.

## Scope

- Change the startup validation projection and response type.
- Add a regression that forbids full-vector projection in this path.
- Measure deployed startup and operation-receipt recovery.

## Uncomfortable fact

This removes the transfer amplification, but it does not reduce SurrealDB's
cost of visiting every embedded record. A future cardinality increase may need
a persisted dimension invariant or database-side mismatch predicate.
