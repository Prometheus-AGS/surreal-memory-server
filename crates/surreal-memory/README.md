# surreal-memory

Embeddable SurrealDB-backed agent memory library for AI applications.

[![Crates.io](https://img.shields.io/crates/v/surreal-memory.svg)](https://crates.io/crates/surreal-memory)
[![Docs.rs](https://docs.rs/surreal-memory/badge.svg)](https://docs.rs/surreal-memory)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Features

| Feature | Description |
|---------|-------------|
| **Knowledge Graph** | Entities, relations, semantic search (HNSW), BM25 full-text, and Graph-RAG traversal (`find_path`, `expand_neighbors`, `get_related`) |
| **Scoped Memory** | mem0-compatible API — 4 memory types (Episodic, Semantic, Procedural, Associative), 3 scopes (User, Session, Agent), full history |
| **Hybrid Search** | Weighted BM25 + HNSW vector score merging |
| **TaskStreams** | Named long-running task contexts with model-aware token budgeting and rolling auto-summarization |
| **Mindmaps** | 5 map types (Radial, Concept, Argument, Tree, Temporal), export to JSON/Mermaid/Markdown, persona auto-generation |
| **Model Profiles** | Built-in token budget registry for GPT-4o, Claude 3.5, Gemini 1.5/2.0 Pro, Llama, Mistral |
| **Zero-loss migrations** | Versioned schema evolution — safe for production upgrades |

## Quickstart

```toml
# Cargo.toml
[dependencies]
surreal-memory = "0.1"
```

```rust
use surreal_memory::{SurrealStorage, Memory, MemoryStorage};
use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = SurrealConfig {
        mode: SurrealMode::Embedded,
        embedded_path: Some("./data/memory.db".into()),
        namespace: "myapp".into(),
        database: "memory".into(),
        ..Default::default()
    };

    let embedding_service = /* create your provider */;
    let storage = SurrealStorage::new(&config, Arc::from(embedding_service)).await?;

    // Add a memory
    let memory = Memory::new(
        "The user prefers dark mode".to_string(),
        Some("user_123".to_string()),
        None, // agent_id
        None, // session_id
        vec!["preferences".to_string()],
    );
    let stored = storage.add_memory(memory).await?;

    // Search memories
    let results = storage.search_memories("UI preferences", Some("user_123"), None, None, None, 5).await?;

    // Graph-RAG: expand entity neighborhood
    let graph = storage.expand_neighbors("Alice", 2, 50).await?;

    Ok(())
}
```

## Model Context Profiles

```rust
use surreal_memory::{profile_for, MODEL_PROFILES};

let profile = profile_for("claude-3-5-sonnet");
println!("Budget: {} tokens", profile.budget()); // 184_000
println!("Auto-summarize threshold: {}", profile.summarization_threshold()); // 147_200
```

## Graph-RAG Traversal

```rust
// Find shortest path between two entities
let paths = storage.find_path("Alice", "Bob", 4).await?;

// Expand 2-hop neighborhood
let subgraph = storage.expand_neighbors("Alice", 2, 50).await?;

// Get all entities Alice WORKS_AT
let colleagues = storage.get_related("Alice", Some("WORKS_AT"), "out", 20).await?;
```

## TaskStreams with Auto-Summarization

```rust
use surreal_memory::TaskStream;

// Create a task stream for a research session
let stream = storage.create_task_stream(TaskStream::new(
    "research-llm-papers",
    Some("LLM paper review session".into()),
    Some("agent_1".into()),
    Some("user_123".into()),
)).await?;

// Auto-summarize when it gets full
storage.auto_summarize_task_stream("research-llm-papers", Some("user_123"), Some("agent_1"), "gpt-4o").await?;
```

## License

MIT — see [LICENSE](../../LICENSE)
