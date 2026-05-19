//! # surreal-memory
//!
//! Embeddable SurrealDB-backed agent memory library providing:
//! - **Knowledge graph** — entities, relations, semantic search, graph traversal
//! - **Scoped memory** — multi-level (Global, Agent, User, Session, Task) mem0-compatible
//! - **TaskStreams** — named, long-running task contexts with model-aware token budgeting
//! - **Mindmaps** — structured visual knowledge (radial, concept, argument, tree, temporal)
//! - **Model profiles** — built-in token budget registry for GPT-4o, Claude, Gemini, and more
//! - **Zero-loss migrations** — versioned schema evolution via `MigrationRunner`

pub mod embeddings;
pub mod entity;
pub mod memory;
pub mod mindmap;
pub mod model_profiles;
#[cfg(feature = "palace")]
pub mod palace;
pub mod storage;
pub mod task_step;
pub mod task_stream;

// Re-exports for ergonomic top-level usage
pub use embeddings::{EmbeddingProvider, EmbeddingService};
pub use entity::{Entity, KnowledgeGraph, Relation, SemanticSearchResult};
pub use memory::{Memory, MemoryHistory, MemoryScope, MemoryType};
pub use mindmap::{ExportFormat, MapType, MindMap, MindMapEdge, MindMapNode};
pub use model_profiles::{MODEL_PROFILES, ModelProfile, profile_for};
pub use storage::MemoryStorage;
pub use storage::surreal::{RetryConfig, SurrealConfig, SurrealStorage};
pub use task_step::{TaskStep, TaskStepStatus};
pub use task_stream::{ContextWindow, TaskStream, TaskStreamStatus};

#[cfg(feature = "palace")]
pub use palace::{
    FastEmbedService, HitSource, PalaceAdapter, PalaceContext, PalaceStatus, PalaceStorage,
    UnifiedHit,
};
