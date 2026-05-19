# write-path Spec Delta — Bound Embedding Model Loading

## ADDED Requirements

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
