# consumer-sync Specification

## Purpose
TBD - created by archiving change commit-and-push-library. Update Purpose after archive.
## Requirements
### Requirement: Library changes SHALL be pushed before a consumer resync

A downstream consumer that pins `surreal-memory` by a git branch SHALL only be
resynced against commits that exist on the remote branch. Uncommitted or
unpushed library work SHALL NOT be a resync target, because `cargo update`
cannot resolve a working tree.

#### Scenario: Resync target exists on the remote
- **GIVEN** a consumer pins `surreal-memory` by `branch = "main"`
- **WHEN** the consumer runs `cargo update -p surreal-memory`
- **THEN** the resolved commit is a real pushed commit on `main`, not a local
  working-tree state

### Requirement: The library test build SHALL be green before it becomes a resync target

The `surreal-memory` library SHALL pass its own `cargo check --tests` before it
is pushed as a resync target, so consumers do not resync onto a library whose
test build is red.

#### Scenario: Library test build verified before push
- **WHEN** the library is about to be pushed for a consumer resync
- **THEN** `cargo check --workspace --features embedded,metal --tests` passes

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

