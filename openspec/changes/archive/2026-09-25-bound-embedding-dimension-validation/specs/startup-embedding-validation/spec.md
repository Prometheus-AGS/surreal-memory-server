# startup-embedding-validation Specification

## ADDED Requirements

### Requirement: Startup dimension validation has bounded response shape

The server SHALL validate stored embedding dimensions by projecting each
embedding's dimension inside the database. The validation response SHALL NOT
contain the embedding vectors themselves.

#### Scenario: Existing dimensions match

- **WHEN** the server validates stored entity and memory embeddings at startup
- **THEN** it requests only record IDs and computed dimensions
- **AND** startup proceeds without transferring full vectors

#### Scenario: Existing dimension conflicts

- **WHEN** a stored embedding dimension differs from the active provider
- **THEN** startup fails with the table, record identity, actual dimension, and
  expected dimension
