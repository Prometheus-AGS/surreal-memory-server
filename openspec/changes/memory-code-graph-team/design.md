# Design

Use explicit Compass includes for src and crates/surreal-memory/src Rust files; exclude the contract exporter under src/bin and standalone tests. Preserve feature-gated source and disclose inline test, trait, macro, SQL and process-boundary limitations. Use local SQLite graph storage, low inference and no semantic enrichment.

The memory-core team separates consumer/API planning, durable operation execution, library storage/schema, embeddings/retrieval, transport/security and independent verification. A lead assigns ownership before concurrent edits; small tasks use one implementer and one reviewer. Project models are native harness choices, not translated provider IDs. Definitions do not activate agents.

Reuse audited local skills, research official additions, and preserve existing SurrealDB skills. Keep credentials out of artifacts. MiniMax uses a project data directory and the Node installation owning mcode. Existing KBD state remains authoritative.

Record source revision, original file hashes, graph coverage/quality, exact MCP command exchange, export receipts, native syntax/discovery checks and review outcomes. Validate this tooling without compiling or launching the application.
