# Design: Database-side dimension projection

## Decision

Use `array::len(embedding) AS dimension` in the existing table-specific startup
queries and deserialize the result as an integer. SurrealDB 3.2 supports
`array::len(array) -> number`; the exact query was also exercised against the
deployed database before implementation.

## Preserved behavior

- Both `entity` and `memory` rows with embeddings are checked.
- A mismatched row still identifies its record and actual dimension.
- Unsupported table names still fail before issuing a query.
- Index metadata and index-definition behavior are unchanged.

## Evidence target

The production process must bind the REST API without transferring full vectors
during validation, and the learning worker must resume durable receipt recovery.
