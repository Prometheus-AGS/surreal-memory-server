# consumer-sync Spec Delta — Resync the librefang Consumer

## ADDED Requirements

### Requirement: A wrapping consumer SHALL verify its resync with a real build

A consumer that only wraps `SurrealStorage` SHALL verify its resync with a real
`cargo build` and `cargo test` run rather than assuming no code change is
needed, even when it does not implement `MemoryStorage`.

#### Scenario: librefang resyncs without code change
- **GIVEN** librefang wraps `surreal_memory::SurrealStorage` and does not
  implement `MemoryStorage`
- **WHEN** librefang is resynced to the updated library
- **THEN** `cargo build` + `cargo test` pass in the librefang repo, and any
  drift the build surfaces is fixed rather than ignored

### Requirement: Library migrations SHALL be verified against a populated consumer DB

A consumer resync that pulls new schema migrations SHALL verify those
migrations apply cleanly against an existing populated embedded database, not
only a fresh one.

#### Scenario: Migrations apply to a populated librefang database
- **GIVEN** the resynced library carries pending migrations
- **WHEN** librefang starts against a populated embedded database
- **THEN** the migrations apply cleanly with no data loss
