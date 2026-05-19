# consumer-sync Spec Delta — Resync the UAR Consumer

## ADDED Requirements

### Requirement: A consumer SHALL compile and test green against the resynced library

After a `surreal-memory` resync, a consumer repo SHALL build and pass its own
test suite against the updated library. Stale call sites left over from a
changed library API SHALL be corrected as part of the resync.

#### Scenario: UAR compiles against the scoped TaskStream API
- **GIVEN** the `surreal-memory` TaskStream methods take `user_id`/`agent_id`
  scope arguments
- **WHEN** UAR is resynced to that library
- **THEN** every UAR call site supplies the scope arguments and
  `cargo build` + `cargo test` pass in the UAR repo

### Requirement: A consumer resync SHALL refresh and commit its lockfile

A consumer resync SHALL run `cargo update -p surreal-memory` and commit the
resulting `Cargo.lock` so the build is reproducible.

#### Scenario: UAR lockfile pins the resynced commit
- **WHEN** UAR is resynced
- **THEN** `Cargo.lock` pins the pushed library commit and the lock change is
  committed in the UAR repo
