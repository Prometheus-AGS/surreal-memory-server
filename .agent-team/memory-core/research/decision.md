# Research and team decision — 2026-09-24

Observed source revision: dd7fdcd6d8974af4059d1d51401bd33ae29f65db. Original dirty paths and hashes are in baseline.json. Active KBD position remains surrealdb-connection-architecture, ready, 13/17; OpenSpec also contains unfinished connection, librefang-sync and pin-policy changes. No existing phase was advanced.

The workspace manifests identify a service crate and the embeddable surreal-memory library. Root src/main.rs starts the API, MCP and TTL worker and supports the embedding-executor subprocess. src/api/mod.rs creates OperationService and merges routes. src/operations.rs implements durable receipts and recovery; src/executor.rs supervises model work. Library storage/migrations and public models are shared consumer contracts. Embeddings and optional palace have distinct dimensions and dependencies. The contract-export binary writes documentation and is excluded from runtime scope. All 43 selected files were extracted. Inline cfg(test) modules remain in this file-level graph, including 134 test relations. Trait dispatch, macros, SQL and process boundaries require additional evidence.

Alternatives considered:

1. One generalist: appropriate for narrow maintenance, insufficient as the only recurring responsibility map for durable execution, migrations and retrieval quality.
2. Copy Boss's ten-role desktop team: rejected because this repository has no renderer/design system and should not inherit desktop roles or tools.
3. Seven reusable roles with at most four active: selected. Product owns external contract usability and consumer coordination; runtime/storage/retrieval/transports map to real subsystems; lead assigns shared files; verifier is independent. Security is a transport responsibility with cross-subsystem review, not a speculative rewrite mandate.

Compass evidence supporting the split includes main→run_api_server, build_router→OperationService::start, payload_hash→digest_hex, run_embedding_executor→execute_request, and MCP tool→run_with_progress. See .compass/verification.json for source anchors and actual MCP evidence. Storage uses ArcSwap now; historic root instructions are retained with a current-source clarification. UAR Cargo.toml points to a vendored library; root notes claiming a direct git pin are historical. Boss workspaceMcp.ts configures a separate memory MCP endpoint. These are source-evidenced contracts, not invented cross-graph edges.

Firecrawl searches:

- `site:github.com surrealdb agent skills Rust SDK surrealql vector SKILL.md` found the community [24601/surreal-skills](https://github.com/24601/surreal-skills) and official ecosystem references. Not installed: broad overlap with existing SurrealDB guidance and extra unrelated SDKs.
- `site:github.com/surrealdb "skills" "SKILL.md" surrealql` found [official agent-skills](https://github.com/surrealdb/agent-skills). Its current README adds performance, functions, CLI and other topics beyond the existing local inventory.

Downloaded only official surrealql-performance plus its three references and MIT license, pinned to commit e5eadc5901337f20ad06ab5acf48d8f83bbfc961. All downloaded files were read before installation. There are no executable scripts, hooks, credentials or install-time commands. It addresses local observed query/index/latency concerns and encourages EXPLAIN evidence. Latest-stable advice is explicitly overridden by this project's pins; examples have not been executed or certified against 3.2.4. SurrealKit was not installed: this project already has a Rust migration runner. Python/JS SDK skills were not added for a Rust service.

Context7 resolved SurrealDB documentation and queried the official /surrealdb/docs.surrealdb.com library for Rust sharing, clone sessions and embedded engines. [Rust multi-tenancy docs](https://github.com/surrealdb/docs.surrealdb.com/blob/main/src/content/reference/rust/concepts/multi-tenancy.mdx) describe independent cloned sessions sharing a connection. Treat current docs as version-sensitive and confirm pinned SDK source before code changes; this research changes no SDK behavior.

Ten additions comprise creator/manage/models/handoff, prometheus-rust-workspace, Rust baseline/async/MCP, API design and official SurrealQL performance. Creator consumer runtime is reused from the reviewed Boss installation, Rust skills from reviewed UAR/local packages; provenance and all installed file hashes are in skills-lock.json. Existing surrealql, surrealdb-vector, surrealdb-expert and surrealdb-python remain byte-preserved. Source/document review is not a proof of every example's correctness. No downloaded code, model, application or database was executed.

Harness contracts are in the bundled creator references/native-harnesses.md, inspected 2026-09-24. Same-day Boss/UAR validation informed the launcher and native formats; project-specific discovery and parse results are recorded separately in verification.json. mcode's active data-directory discovery and Node/native-SQLite constraint are handled without global configuration changes. Provider credentials and live inference remain separate from definition installation.
