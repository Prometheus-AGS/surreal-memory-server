# consumer-sync Spec Delta — Commit and Push the Library

## ADDED Requirements

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
