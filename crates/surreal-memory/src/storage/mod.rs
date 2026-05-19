//! Storage trait and SurrealDB implementation for the surreal-memory library.

use anyhow::Result;
use async_trait::async_trait;
use std::any::Any;

pub mod migrations;
pub mod surreal;

use crate::entity::{Entity, KnowledgeGraph, Relation, SemanticSearchResult};
use crate::memory::{Memory, MemoryHistory};
use crate::mindmap::{MindMap, MindMapEdge, MindMapNode};
use crate::task_step::{TaskStep, TaskStepStatus};
use crate::task_stream::{ContextWindow, TaskStream};

/// The unified storage interface for the surreal-memory platform.
///
/// Implemented by `SurrealStorage`. All methods are `async` and return
/// `anyhow::Result<T>` for ergonomic error propagation.
#[async_trait]
pub trait MemoryStorage: Send + Sync {
    // ── Knowledge Graph ──────────────────────────────────────────────────────

    async fn create_entity(&self, entity: Entity) -> Result<Entity>;
    async fn create_entities(&self, entities: Vec<Entity>) -> Result<Vec<Entity>>;
    async fn get_entity(&self, name: &str) -> Result<Option<Entity>>;
    async fn update_entity(&self, entity: Entity) -> Result<Entity>;
    async fn delete_entity(&self, name: &str) -> Result<()>;
    async fn search_entities(&self, query: &str) -> Result<Vec<Entity>>;
    async fn create_relation(&self, relation: Relation) -> Result<Relation>;
    async fn create_relations(&self, relations: Vec<Relation>) -> Result<Vec<Relation>>;
    async fn get_relations(&self, entity_name: &str) -> Result<Vec<Relation>>;
    async fn delete_relation(&self, from: &str, to: &str, relation_type: &str) -> Result<()>;
    async fn get_graph(&self) -> Result<KnowledgeGraph>;
    async fn add_observations(
        &self,
        entity_name: &str,
        observations: Vec<String>,
    ) -> Result<Entity>;
    async fn semantic_search(
        &self,
        query: &str,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<SemanticSearchResult>>;

    // ── Scoped Memory (mem0) ─────────────────────────────────────────────────

    async fn add_memory(&self, memory: Memory) -> Result<Memory>;
    async fn get_memory(&self, id: &str) -> Result<Option<Memory>>;
    async fn update_memory(&self, id: &str, content: String) -> Result<Memory>;
    async fn delete_memory(&self, id: &str) -> Result<()>;
    async fn delete_all_memories(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<u64>;
    async fn get_all_memories(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<Memory>>;
    async fn search_memories(
        &self,
        query: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        categories: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<Memory>>;
    async fn get_memory_history(&self, memory_id: &str) -> Result<Vec<MemoryHistory>>;

    /// Hybrid BM25 + HNSW vector search. Scores merged as:
    /// `vector_weight * vec_score + bm25_weight * bm25_score`.
    #[allow(clippy::too_many_arguments)]
    async fn hybrid_search_memories(
        &self,
        query: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        limit: usize,
        vector_weight: f32,
        bm25_weight: f32,
    ) -> Result<Vec<Memory>>;

    /// Compress memories older than `older_than_days` into a single summary.
    async fn compress_memories(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        older_than_days: u32,
    ) -> Result<Option<Memory>>;

    /// Auto-extract semantic memories from raw conversation messages.
    /// messages: `[{"role": "user"|"assistant", "content": "..."}]`
    async fn add_memories_from_conversation(
        &self,
        messages: Vec<serde_json::Value>,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<Memory>>;

    /// Mark memories with `valid_until < now()` as expired. Returns count.
    async fn expire_stale_memories(&self) -> Result<u64>;

    // ── TaskStreams ───────────────────────────────────────────────────────────

    async fn create_task_stream(&self, stream: TaskStream) -> Result<TaskStream>;
    async fn get_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Option<TaskStream>>;
    async fn add_to_task_stream(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        memory: Memory,
    ) -> Result<Memory>;
    async fn get_context_for_task(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        model_name: &str,
        max_tokens: Option<u64>,
    ) -> Result<ContextWindow>;
    async fn list_task_streams(
        &self,
        agent_id: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<TaskStream>>;
    async fn delete_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<()>;
    async fn archive_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<TaskStream>;
    /// Pause an active `TaskStream`, suspending further additions without archiving it.
    async fn pause_task_stream(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<TaskStream>;

    // ── TaskSteps ────────────────────────────────────────────────────────────

    /// Add an ordered step to a stream. Idempotent on `idempotency_key`: if a
    /// step with that key already exists, the existing step is returned and no
    /// duplicate is created.
    async fn add_task_step(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        step: TaskStep,
    ) -> Result<TaskStep>;

    /// Update a step's status, recording result/error and started/completed
    /// timestamps as appropriate. Looked up by `idempotency_key`.
    async fn update_task_step_status(
        &self,
        idempotency_key: &str,
        status: TaskStepStatus,
        result: Option<String>,
        error: Option<String>,
    ) -> Result<TaskStep>;

    /// Return all steps of a stream ordered by `ordinal`.
    async fn get_task_steps(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Vec<TaskStep>>;

    /// Returns the lowest-ordinal step whose status is not `Completed` or
    /// `Skipped`. A `Failed` or `Running` step is returned as current so the
    /// caller can resolve it (retry, skip, or complete) before advancing — a
    /// failed step intentionally blocks progress until resolved.
    async fn get_current_step(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Option<TaskStep>>;

    /// Mark a step `completed`. Idempotent: completing an already-completed
    /// step returns it unchanged without re-applying the result.
    async fn complete_step(
        &self,
        idempotency_key: &str,
        result: Option<String>,
    ) -> Result<TaskStep>;

    // ── Mindmaps ─────────────────────────────────────────────────────────────

    async fn create_mindmap(&self, mindmap: MindMap) -> Result<MindMap>;
    async fn get_mindmap(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Option<MindMap>>;
    async fn add_mindmap_node(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        node: MindMapNode,
    ) -> Result<MindMap>;
    async fn add_mindmap_edge(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        edge: MindMapEdge,
    ) -> Result<MindMap>;
    async fn delete_mindmap_node(
        &self,
        mindmap_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        node_id: &str,
    ) -> Result<MindMap>;
    async fn list_mindmaps(
        &self,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Vec<MindMap>>;
    async fn delete_mindmap(
        &self,
        name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<()>;

    // ── Phase 3: Advanced Context + Graph-RAG ────────────────────────────────

    /// Rolling auto-summarization: compress the oldest half of a TaskStream's
    /// memories when `total_tokens` exceeds the model's summarization threshold.
    /// Returns the new summary memory, or `None` if no action was needed.
    async fn auto_summarize_task_stream(
        &self,
        stream_name: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        model_id: &str,
    ) -> Result<Option<Memory>>;

    /// Non-fatally update the user's persona mindmap after a new memory is added.
    /// Errors are logged but not propagated — `add_memory` must remain fast.
    async fn try_update_persona_mindmap(&self, user_id: &str, memory: &Memory) -> Result<()>;

    /// Find shortest paths between two entities up to `max_depth` hops.
    /// Returns a list of paths, where each path is a Vec of entity names.
    async fn find_path(&self, from: &str, to: &str, max_depth: u8) -> Result<Vec<Vec<String>>>;

    /// Return the subgraph of entities within `depth` hops of `entity_name`.
    async fn expand_neighbors(
        &self,
        entity_name: &str,
        depth: u8,
        limit: usize,
    ) -> Result<KnowledgeGraph>;

    /// Return entities connected to `entity_name` by relations, optionally
    /// filtered by `relation_type` and/or `direction` ("in" | "out" | "both").
    async fn get_related(
        &self,
        entity_name: &str,
        relation_type: Option<&str>,
        direction: &str,
        limit: usize,
    ) -> Result<Vec<Entity>>;

    // ── Phase 4: Temporal Entity History ────────────────────────────────

    /// Return the full observation-change history for an entity, ordered newest-first.
    async fn get_entity_history(&self, name: &str) -> Result<Vec<MemoryHistory>>;

    /// Return the knowledge graph as it existed at or before `before_rfc3339`.
    /// `before_rfc3339` must be an RFC-3339 timestamp string (e.g. "2025-01-01T00:00:00Z").
    async fn get_graph_at_time(&self, before_rfc3339: &str) -> Result<KnowledgeGraph>;

    /// Downcast helper — allows MCP handlers to recover concrete types
    /// (e.g. `SurrealStorage`) from `Arc<dyn MemoryStorage>`.
    fn as_any(&self) -> &dyn Any;
}
