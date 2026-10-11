# Agent Fabric Convergence: surreal-memory planning notes

Date: 2026-09-25

Status: planning input; memory changes require an evidenced UAR integration gap

Source baseline: `4ed6b2c79b9a28b56e53f208fcb85306ec443ae2`

## Role in the convergence

surreal-memory owns scoped context, retrieval, durable memory history, TaskStream context, and memory-specific operation journals. It does not own agent execution, team scheduling, workflow transitions, Cedar authorization, or protected effects.

The convergence plan's C05 keeps the BossFang workflow while delegating a complete run to UAR, and C06/C09 place runtime instances and team state in UAR; C01 must reconcile those statements with the accepted P1 contracts. A team or workflow owner may store selected context and references here after authorization, but the memory record must not become the authority for membership, delegation, approval, or effect completion.

## Current source facts

- `MemoryScope` supports `global`, `agent`, `user`, `session`, and `task`; `Memory` carries optional user, agent, session, and TaskStream identifiers (`crates/surreal-memory/src/memory.rs`). There is no team, organization, representation-grant, membership-revision, or policy-revision field in the current public model.
- `TaskStream` is scoped by optional agent and user identifiers. Migration v18 makes stream-name uniqueness `(agent_id, user_id, name)` (`crates/surreal-memory/src/task_stream.rs`, `crates/surreal-memory/src/storage/migrations/mod.rs`).
- `TaskStep` provides ordered status tracking and an idempotency key. Its module explicitly states that this is durable tracking, not orchestration (`crates/surreal-memory/src/task_step.rs`). It therefore must not be promoted into the UAR team scheduler or a cross-product workflow engine.
- Migration v20 adds a durable `memory_operation` ledger and append-only events/parts; migration v21 journals the supervised embedding executor lifecycle. These records own memory ingestion/executor work, not arbitrary agent runs or external effects (`crates/surreal-memory/src/storage/migrations/mod.rs`).
- `MemoryStorage` exposes scoped memory, TaskStream, TaskStep, mindmap, and graph operations. It has no grant-aware team mailbox or ownership-epoch API (`crates/surreal-memory/src/storage/mod.rs`).
- Repository guidance records the UAR integration convention for CLI agents: `user_id = "anonymous"`, `agent_id` as the primary discriminator, no session for cross-session queries, and a session only for session-specific writes (`docs/lessons.md`, `AGENTS.md`). That convention is evidence for the current single-agent path, not a team-sharing rule.

## Future scoped-memory contract

C06 should first use UAR's own durable instance store and existing thread execution. Its plan already limits surreal-memory API changes to an evidenced gap. Reactivation, inbox ownership, cancellation, fencing, and run state should not be stored as memory merely because this repository already uses SurrealDB.

C09 may need selected team context or artifact namespaces. That sharing must be explicit and narrower than the union of member scopes. The team owner should pass a current grant and selected references through UAR's memory facade; surreal-memory should enforce only the storage contract assigned to it. Exact schema and enforcement placement require a D-MEMORY/UAR contract decision. Reusing one agent id for an entire team or placing team material in `global` scope would erase provenance and is not supported by the present model.

Executive or human-representation context in C17 needs the same discipline. Person-specific memory may inform a disclosed assistant or digital twin, but memory content cannot prove consent, organizational assignment, authority, or approval. Those remain versioned grants enforced at the UAR/Gate boundary.

For Codex CLI, use UAR's existing memory facade and preserve the current scope convention until a reviewed team contract exists. Codex workers may receive selected context and return memory/artifact references; they should not read every teammate's transcript or mutate a shared global stream by default.

## Dependencies and next repository-scoped KBD child

Recommended next child: **D-MEMORY scope-contract assessment for C06/C09**.

This should be an assessment child before implementation. It depends on the accepted UAR P1 memory facade and C01 vocabulary. Its first decision is whether UAR can implement instance and team isolation by composing the current `MemoryStorage` methods. Only a reproduced integration gap should authorize a new public trait method or migration.

If a gap is proven, split implementation into one repository-scoped child with explicit struct, trait, migration, and UAR consumer ownership. Acceptance must use the real UAR consumer and both supported SurrealDB modes; it must show two users can instantiate one definition without leakage, selected team context is visible only under a current grant, revoked membership blocks new retrieval, and existing single-agent scope behavior remains compatible.

## Evidence to preserve in the child plan

- Baseline commit and UAR's exact surreal-memory git revision.
- Existing user/agent/session/task scope behavior and TaskStream uniqueness.
- Proof that a proposed team operation cannot be expressed safely through the existing facade.
- Chosen owner of membership/grant revisions and the minimal reference stored with memory.
- Schema-to-struct migration mapping for every persisted field.
- Real-consumer verification plan with isolated embedded/server resources and one build writer.
- A kickoff recheck of the UAR P1 facade, D-MEMORY receipt, and the single owner for runtime and workflow state.
