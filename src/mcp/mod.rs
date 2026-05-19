use anyhow::Result;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use std::sync::Arc;

use crate::storage::MemoryStorage;

pub mod handlers;
pub mod http;
pub mod progress;

use progress::run_with_progress;

use handlers::{
    AddMemoryParams, AddMindmapEdgeParams, AddMindmapNodeParams, AddObservationsParams,
    AddTaskStepParams, AddToTaskStreamParams, AutoSummarizeTaskStreamParams, CompleteStepParams,
    CompressMemoriesParams, ConversationParams, CreateEntitiesParams, CreateEntityParams,
    CreateMindmapParams, CreateRelationParams, CreateRelationsParams, CreateTaskStreamParams,
    DeleteEntityParams, DeleteMindmapNodeParams, DeleteRelationParams, EntityNameParams,
    ExpandNeighborsParams, ExportMindmapParams, FindPathParams, GenerateIdeationMindmapParams,
    GeneratePersonaMindmapParams, GetContextParams, GetRelatedParams, HybridSearchParams,
    ListMindmapsParams, MemoryHandler, MemoryIdParams, MindmapNameParams, ScopeFilterParams,
    SearchMemoriesParams, SearchParams, SemanticSearchParams, TaskStreamNameParams, TimeParams,
    UpdateMemoryParams, UpdateTaskStepStatusParams,
};

use handlers::{
    PalaceDeleteParams, PalaceHybridSearchParams, PalaceIngestParams, PalaceRecallParams,
    PalaceSearchParams, PalaceWakeUpParams,
};

#[derive(Clone)]
pub struct MemoryMcpServer {
    handler: Arc<MemoryHandler>,
    #[expect(
        dead_code,
        reason = "rmcp's tool_handler macro consumes this field from generated code"
    )]
    tool_router: ToolRouter<MemoryMcpServer>,
}

impl MemoryMcpServer {
    pub fn new(storage: Arc<dyn MemoryStorage>) -> Self {
        let handler = Arc::new(MemoryHandler::new(storage));
        Self {
            handler,
            tool_router: Self::tool_router(),
        }
    }

    pub async fn run(self) -> Result<()> {
        let service = self.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }
}

#[tool_router]
impl MemoryMcpServer {
    #[tool(
        description = "Create a new entity in the knowledge graph (e.g., a person, project, or concept). Requires name, type, and at least one observation."
    )]
    async fn create_entity(
        &self,
        Parameters(params): Parameters<CreateEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_entity(params).await
    }

    #[tool(
        description = "Add new facts or observations to an existing entity. The entity must already exist."
    )]
    async fn add_observations(
        &self,
        Parameters(params): Parameters<AddObservationsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.add_observations(params).await
    }

    #[tool(
        description = "Create a directed relationship between two existing entities (e.g., 'Alice WORKS_AT Acme')."
    )]
    async fn create_relation(
        &self,
        Parameters(params): Parameters<CreateRelationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_relation(params).await
    }

    #[tool(
        description = "Search entities by exact text match on name or type. Use semantic_search instead for meaning-based similarity."
    )]
    async fn search_entities(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.search(params).await
    }

    #[tool(
        description = "Retrieve the complete knowledge graph with all entities and relations. Use for full context or debugging."
    )]
    async fn read_graph(&self) -> Result<CallToolResult, McpError> {
        self.handler.get_graph().await
    }

    #[tool(description = "Retrieve a single entity by exact name.")]
    async fn get_entity(
        &self,
        Parameters(params): Parameters<EntityNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_entity(params).await
    }

    #[tool(
        description = "Update an existing entity exactly. Replaces entity type and observations."
    )]
    async fn update_entity(
        &self,
        Parameters(params): Parameters<CreateEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.update_entity(params).await
    }

    #[tool(description = "Retrieve all relations (incoming and outgoing) for an entity.")]
    async fn get_relations(
        &self,
        Parameters(params): Parameters<EntityNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_relations(params).await
    }

    #[tool(description = "Retrieve the temporal audit log of an entity's changes.")]
    async fn get_entity_history(
        &self,
        Parameters(params): Parameters<EntityNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_entity_history(params).await
    }

    #[tool(
        description = "Retrieve the full knowledge graph as it existed at a past RFC-3339 timestamp."
    )]
    async fn get_graph_at_time(
        &self,
        Parameters(params): Parameters<TimeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_graph_at_time(params).await
    }

    #[tool(
        description = "Permanently delete an entity and automatically remove all its relations."
    )]
    async fn delete_entity(
        &self,
        Parameters(params): Parameters<DeleteEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_entity(params).await
    }

    #[tool(
        description = "Delete a specific relation between two entities. Requires exact from, to, and relation_type."
    )]
    async fn delete_relation(
        &self,
        Parameters(params): Parameters<DeleteRelationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_relation(params).await
    }

    #[tool(
        description = "Find entities by meaning similarity using vector embeddings. Best for natural language queries like 'who works on distributed systems?'"
    )]
    async fn semantic_search(
        &self,
        Parameters(params): Parameters<SemanticSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.semantic_search(params).await
    }

    // ── Batch Knowledge Graph ─────────────────────────────────────────────────

    #[tool(description = "Create multiple entities at once in the knowledge graph.")]
    async fn create_entities(
        &self,
        Parameters(params): Parameters<CreateEntitiesParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_entities(params).await
    }

    #[tool(description = "Create multiple relations at once between existing entities.")]
    async fn create_relations(
        &self,
        Parameters(params): Parameters<CreateRelationsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_relations(params).await
    }

    // ── Scoped Memory ─────────────────────────────────────────────────────────

    #[tool(
        description = "Add a new scoped memory. Supports user/agent/session scoping and semantic deduplication."
    )]
    async fn add_memory(
        &self,
        Parameters(params): Parameters<AddMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.add_memory(params).await
    }

    #[tool(description = "Retrieve a specific memory by its record ID.")]
    async fn get_memory(
        &self,
        Parameters(params): Parameters<MemoryIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_memory(params).await
    }

    #[tool(
        description = "Update the content of an existing memory. Creates a MemoryHistory audit record."
    )]
    async fn update_memory(
        &self,
        Parameters(params): Parameters<UpdateMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.update_memory(params).await
    }

    #[tool(description = "Delete a specific memory by ID.")]
    async fn delete_memory(
        &self,
        Parameters(params): Parameters<MemoryIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_memory(params).await
    }

    #[tool(description = "Delete all memories matching the given user/agent/session scope.")]
    async fn delete_all_memories(
        &self,
        Parameters(params): Parameters<ScopeFilterParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_all_memories(params).await
    }

    #[tool(description = "List all memories for a given user/agent/session scope.")]
    async fn get_all_memories(
        &self,
        Parameters(params): Parameters<ScopeFilterParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_all_memories(params).await
    }

    #[tool(
        description = "Semantic search over scoped memories. Filters by user/agent/session and optional categories."
    )]
    async fn search_memories(
        &self,
        Parameters(params): Parameters<SearchMemoriesParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.search_memories(params).await
    }

    #[tool(description = "Get the full edit history of a memory (all versions).")]
    async fn get_memory_history(
        &self,
        Parameters(params): Parameters<MemoryIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_memory_history(params).await
    }

    // ── TaskStreams ───────────────────────────────────────────────────────────

    #[tool(
        description = "Create a named task stream for tracking memories across a long-running task (e.g. a coding session)."
    )]
    async fn create_task_stream(
        &self,
        Parameters(params): Parameters<CreateTaskStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_task_stream(params).await
    }

    #[tool(description = "Get the metadata of a task stream by name.")]
    async fn get_task_stream(
        &self,
        Parameters(params): Parameters<TaskStreamNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_task_stream(params).await
    }

    #[tool(description = "Add a memory to a task stream.")]
    async fn add_to_task_stream(
        &self,
        Parameters(params): Parameters<AddToTaskStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.add_to_task_stream(params).await
    }

    #[tool(
        description = "Get a model-aware context window for a task stream, respecting the model's token budget."
    )]
    async fn get_context_for_task(
        &self,
        Parameters(params): Parameters<GetContextParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_context_for_task(params).await
    }

    #[tool(description = "List all active task streams, optionally filtered by agent/user.")]
    async fn list_task_streams(
        &self,
        Parameters(params): Parameters<ScopeFilterParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.list_task_streams(params).await
    }

    #[tool(description = "Archive a task stream by name (marks it as inactive).")]
    async fn archive_task_stream(
        &self,
        Parameters(params): Parameters<TaskStreamNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.archive_task_stream(params).await
    }

    #[tool(description = "Pause a task stream by name without archiving it.")]
    async fn pause_task_stream(
        &self,
        Parameters(params): Parameters<TaskStreamNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.pause_task_stream(params).await
    }

    // ── TaskSteps ─────────────────────────────────────────────────────────────

    #[tool(
        description = "Add an ordered, status-tracked step to a task stream. Idempotent on idempotency_key — re-adding with the same key returns the existing step."
    )]
    async fn add_task_step(
        &self,
        Parameters(params): Parameters<AddTaskStepParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.add_task_step(params).await
    }

    #[tool(
        description = "Update a task step's status (pending/running/completed/failed/skipped), recording result/error and timestamps."
    )]
    async fn update_task_step_status(
        &self,
        Parameters(params): Parameters<UpdateTaskStepStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.update_task_step_status(params).await
    }

    #[tool(description = "List all steps of a task stream, ordered by ordinal.")]
    async fn get_task_steps(
        &self,
        Parameters(params): Parameters<TaskStreamNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_task_steps(params).await
    }

    #[tool(
        description = "Get the current (lowest-ordinal not-yet-completed) step of a task stream. Use to resume a multi-step process."
    )]
    async fn get_current_step(
        &self,
        Parameters(params): Parameters<TaskStreamNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_current_step(params).await
    }

    #[tool(
        description = "Mark a task step completed. Idempotent — completing an already-completed step returns it without re-applying the result."
    )]
    async fn complete_step(
        &self,
        Parameters(params): Parameters<CompleteStepParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.complete_step(params).await
    }

    // ── Mindmaps ─────────────────────────────────────────────────────────────

    #[tool(
        description = "Create a new mindmap (radial, concept, argument, tree, or temporal). Starts with a single root node."
    )]
    async fn create_mindmap(
        &self,
        Parameters(params): Parameters<CreateMindmapParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.create_mindmap(params).await
    }

    #[tool(description = "Retrieve a mindmap by name, optionally scoped to a user.")]
    async fn get_mindmap(
        &self,
        Parameters(params): Parameters<MindmapNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_mindmap(params).await
    }

    #[tool(description = "List all mindmaps, optionally filtered by user_id or agent_id.")]
    async fn list_mindmaps(
        &self,
        Parameters(params): Parameters<ListMindmapsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.list_mindmaps(params).await
    }

    #[tool(description = "Add a node to an existing mindmap.")]
    async fn add_mindmap_node(
        &self,
        Parameters(params): Parameters<AddMindmapNodeParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        run_with_progress(
            &ctx,
            "add_mindmap_node",
            self.handler.add_mindmap_node(params),
        )
        .await
    }

    #[tool(description = "Add a directed or undirected edge between two nodes in a mindmap.")]
    async fn add_mindmap_edge(
        &self,
        Parameters(params): Parameters<AddMindmapEdgeParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        run_with_progress(
            &ctx,
            "add_mindmap_edge",
            self.handler.add_mindmap_edge(params),
        )
        .await
    }

    #[tool(description = "Delete a node (and its incident edges) from a mindmap.")]
    async fn delete_mindmap_node(
        &self,
        Parameters(params): Parameters<DeleteMindmapNodeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_mindmap_node(params).await
    }

    #[tool(description = "Delete an entire mindmap.")]
    async fn delete_mindmap(
        &self,
        Parameters(params): Parameters<MindmapNameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.delete_mindmap(params).await
    }

    #[tool(description = "Export a mindmap as JSON, Mermaid diagram, or Markdown.")]
    async fn export_mindmap(
        &self,
        Parameters(params): Parameters<ExportMindmapParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.export_mindmap(params).await
    }

    #[tool(
        description = "Auto-generate a radial persona mindmap from all memories associated with a user."
    )]
    async fn generate_persona_mindmap(
        &self,
        Parameters(params): Parameters<GeneratePersonaMindmapParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.generate_persona_mindmap(params).await
    }

    #[tool(
        description = "Create an empty ideation mindmap for a given topic and map type, ready to populate."
    )]
    async fn generate_ideation_mindmap(
        &self,
        Parameters(params): Parameters<GenerateIdeationMindmapParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.generate_ideation_mindmap(params).await
    }

    // ── Hybrid Search + mem0 Advanced ────────────────────────────────────────

    #[tool(
        description = "Hybrid BM25 full-text + HNSW vector search over memories with configurable weight ratio."
    )]
    async fn hybrid_search_memories(
        &self,
        Parameters(params): Parameters<HybridSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.hybrid_search_memories(params).await
    }

    #[tool(description = "Compress memories older than N days into a single summary memory.")]
    async fn compress_memories(
        &self,
        Parameters(params): Parameters<CompressMemoriesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        run_with_progress(
            &ctx,
            "compress_memories",
            self.handler.compress_memories(params),
        )
        .await
    }

    #[tool(
        description = "Auto-extract and store memories from raw conversation messages (role+content array)."
    )]
    async fn add_memories_from_conversation(
        &self,
        Parameters(params): Parameters<ConversationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.add_memories_from_conversation(params).await
    }

    // ── Phase 3: Advanced Context + Graph-RAG ────────────────────────────────

    #[tool(
        description = "Trigger rolling auto-summarization on a TaskStream when total_tokens exceeds the model budget threshold. Compresses the oldest half of memories into a summary."
    )]
    async fn auto_summarize_task_stream(
        &self,
        Parameters(params): Parameters<AutoSummarizeTaskStreamParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        run_with_progress(
            &ctx,
            "auto_summarize_task_stream",
            self.handler.auto_summarize_task_stream(params),
        )
        .await
    }

    #[tool(
        description = "Find shortest paths between two entities in the knowledge graph. Returns up to 5 paths with up to max_depth hops (default 4)."
    )]
    async fn find_path(
        &self,
        Parameters(params): Parameters<FindPathParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.find_path(params).await
    }

    #[tool(
        description = "Return the subgraph of entities within N hops of an entity (graph-RAG neighborhood expansion)."
    )]
    async fn expand_neighbors(
        &self,
        Parameters(params): Parameters<ExpandNeighborsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.expand_neighbors(params).await
    }

    #[tool(
        description = "Return entities related to an entity, optionally filtered by relation_type and direction (in/out/both)."
    )]
    async fn get_related(
        &self,
        Parameters(params): Parameters<GetRelatedParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.get_related(params).await
    }

    // ── Palace (Memory Palace) ───────────────────────────────────────────────

    #[tool(
        description = "Load palace identity context (L0 identity + L1 essential story). Call at conversation start to prime the agent with persistent identity."
    )]
    async fn palace_wake_up(
        &self,
        Parameters(params): Parameters<PalaceWakeUpParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_wake_up(params).await
    }

    #[tool(
        description = "On-demand recall of palace drawers (L2 layer). Returns formatted context filtered by wing/room."
    )]
    async fn palace_recall(
        &self,
        Parameters(params): Parameters<PalaceRecallParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_recall(params).await
    }

    #[tool(
        description = "Deep semantic search across palace drawers (L3 layer). Returns formatted context text matching the query."
    )]
    async fn palace_search(
        &self,
        Parameters(params): Parameters<PalaceSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_search(params).await
    }

    #[tool(
        description = "Ingest content into the memory palace. Stores it as a drawer in the specified wing/room/hall with importance weighting."
    )]
    async fn palace_ingest(
        &self,
        Parameters(params): Parameters<PalaceIngestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        run_with_progress(&ctx, "palace_ingest", self.handler.palace_ingest(params)).await
    }

    #[tool(description = "Delete a drawer from the memory palace by its record ID.")]
    async fn palace_delete(
        &self,
        Parameters(params): Parameters<PalaceDeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_delete(params).await
    }

    #[tool(
        description = "Get palace status: total drawers, wings, rooms, and identity loading state."
    )]
    async fn palace_status(&self) -> Result<CallToolResult, McpError> {
        self.handler.palace_status().await
    }

    #[tool(
        description = "Unified hybrid search across memories, entities, and palace drawers. Results are merged via Reciprocal Rank Fusion scoring."
    )]
    async fn palace_hybrid_search(
        &self,
        Parameters(params): Parameters<PalaceHybridSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.handler.palace_hybrid_search(params).await
    }
}

#[tool_handler]
impl ServerHandler for MemoryMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Knowledge graph memory server with scoped memory, TaskStreams, Mindmaps, and Graph-RAG. \
                Knowledge graph: create_entity/create_entities, add_observations, create_relation/create_relations, \
                read_graph, search_entities, semantic_search, delete_entity, delete_relation. \
                Graph-RAG traversal: find_path, expand_neighbors, get_related. \
                Scoped memory (mem0-compatible): add_memory, get_memory, update_memory, delete_memory, \
                delete_all_memories, get_all_memories, search_memories, get_memory_history, \
                hybrid_search_memories, compress_memories, add_memories_from_conversation. \
                TaskStreams: create_task_stream, get_task_stream, add_to_task_stream, \
                get_context_for_task, list_task_streams, archive_task_stream, auto_summarize_task_stream, \
                pause_task_stream. \
                TaskSteps: add_task_step, update_task_step_status, get_task_steps, get_current_step, \
                complete_step. \
                Mindmaps: create_mindmap, get_mindmap, list_mindmaps, add_mindmap_node, \
                add_mindmap_edge, delete_mindmap_node, delete_mindmap, export_mindmap, \
                generate_persona_mindmap, generate_ideation_mindmap.",
            )
    }
}
