# Current Waypoint

- **Phase**: surrealdb-connection-architecture
- **Status**: planned (assessment + plan complete; not yet executing)
- **Change backend**: OpenSpec
- **Active change**: [`fix-surrealdb-connection-architecture`](../openspec/changes/fix-surrealdb-connection-architecture/proposal.md)

## Paused phase

- `uar-consumer-resync` — was `executing`; paused to address this
  higher-priority architectural defect first. Resume after the connection
  refactor lands, since UAR is the primary consumer of `SurrealStorage`.

## Phase artifacts

- [Assessment](phases/surrealdb-connection-architecture/assessment.md)
- [Plan](phases/surrealdb-connection-architecture/plan.md)
- [Progress](phases/surrealdb-connection-architecture/progress.json)
- [OpenSpec proposal](../openspec/changes/fix-surrealdb-connection-architecture/proposal.md)
- [OpenSpec tasks](../openspec/changes/fix-surrealdb-connection-architecture/tasks.md)

## Ordered changes (execute strictly in order)

1. **load-repro-harness** — baseline measurement (gates everything below)
2. **arcswap-connection-cell** — CRITICAL: removes blocking lock from async hot path
3. **config-query-timeout** — push deadline into SDK layer
4. **typed-error-retry-classification** — surrealdb::Error variants, not string substrings
5. **workload-isolated-sessions** — CONDITIONAL on user §6 answers
6. **embedded-mode-semaphore + framing** — bound concurrency, honest docs
7. **docs: skill references** — partially done in plan turn (CLAUDE.md, AGENTS.md updated)

## Next step

```
/kbd-execute fix-surrealdb-connection-architecture
```

This dispatches **Change 1 (load-repro harness)** first. Do not skip it —
without a baseline the later correctness claims are unfalsifiable.

## Open questions still in flight

See [assessment §6](phases/surrealdb-connection-architecture/assessment.md).
Do not block Changes 1–4 and 7. Required before Changes 5–6 ship.
