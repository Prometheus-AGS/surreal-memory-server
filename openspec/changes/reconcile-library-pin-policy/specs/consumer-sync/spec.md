# consumer-sync Spec Delta — Reconcile the Library Pin Policy

## ADDED Requirements

### Requirement: The library pin policy SHALL be explicit and documented

The pin style each consumer uses for `surreal-memory` SHALL be a deliberate,
recorded decision, and the surreal-memory-server documentation SHALL match the
pin style actually in use.

#### Scenario: Pin policy matches documentation
- **WHEN** a consumer's `Cargo.toml` pin style is compared to
  surreal-memory-server `CLAUDE.md` rule #7
- **THEN** they describe the same pin policy

### Requirement: A rev pin SHALL reference a verified commit

If a consumer pins `surreal-memory` by `rev`, that rev SHALL be a commit the
consumer has been built and tested against, not an arbitrary commit.

#### Scenario: Rev pin is a verified commit
- **GIVEN** a consumer is moved to a `rev` pin
- **WHEN** the rev is chosen
- **THEN** it is the commit that the consumer's `cargo build` + `cargo test`
  passed against
