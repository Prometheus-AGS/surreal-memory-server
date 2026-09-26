# Task routing and ownership

Read both root instruction files. Before edits inspect the affected source, canonical schema and current call graph. Restore the existing KBD position; this setup does not finish the connection-architecture phase.

| Trigger | Primary role | Consult before completion | Skills |
| --- | --- | --- | --- |
| Multi-subsystem change, build/dependency or release plan | lead | relevant specialists, verifier | creator, manage, models, handoff, prometheus-rust-workspace |
| API/library behavior, consumer compatibility, operator workflow | product | lead, transports, storage | api-and-interface-design, handoff |
| Acceptance, replay, recovery, SSE or executor lifecycle | runtime | storage, retrieval, transports | Rust baseline + async, handoff |
| Persisted field, query, schema, connection or library API | storage | product, retrieval | Rust baseline + async, surrealql, surrealdb-expert, surrealql-performance |
| Model/provider, chunking, vector search, palace, retrieval latency | retrieval | storage, runtime | Rust baseline + async, surrealdb-vector, surrealql-performance |
| MCP/REST/A2A, coercion, permissions or untrusted inputs | transports | product, runtime, verifier | Rust baseline, rust-mcp-server-generator, api-and-interface-design |
| Completed implementation/configuration or release claim | verifier | fresh evidence, no producer verdict | Rust baseline when applicable, handoff, review protocol |

`team.json` records write domains. Read access is broader. Assign concrete files before concurrent work, including inline tests within owned Rust files. Tests/CI/deployment files, root instructions, `.mcp.json`, team manifests and any uncovered path need a named writer assigned by the lead. Documentation ownership does not authorize changes to the pre-existing `.prometheus/knowledge` store. Changes in `storage/surreal.rs` for retrieval remain a storage writer's work; ledger DDL in `operations.rs` remains a runtime writer's work with storage review. Root exports and manifests need coordinated integration. Reviewers own findings only; if they implement a fix, use another reviewer for that fix.

Choose the smallest useful team. Seven definitions cover recurring responsibilities, not a mandatory seven-agent workflow. One implementer handles a narrow bug. Use product plus relevant specialists for contract changes. Durable ingestion spanning ledger, executor and storage warrants explicit contract handoffs. Security review is required at actual external input, identity, tool execution and data isolation boundaries; do not create speculative security refactors.

Model prices and capabilities remain unknown unless discovered. No filename, role ID or ownership glob enforces permissions; the harness owns sandboxing, approvals and spawning.
