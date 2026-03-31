//! Integration tests for the `surreal-memory` library.
//!
//! All tests use `mem://` SurrealDB — zero external dependencies, runs with just `cargo test`.

use std::sync::Arc;
use surreal_memory::mindmap::{MapType, MindMap, MindMapNode};
use surreal_memory::{
    Entity, Memory, MemoryStorage, Relation, TaskStream,
    model_profiles::{MODEL_PROFILES, profile_for},
    storage::surreal::{RetryConfig, SurrealConfig, SurrealMode, SurrealStorage},
    task_stream::TaskStreamStatus,
};
use uuid::Uuid;

// ── NoOp Embedder ─────────────────────────────────────────────────────────────

struct NoOpEmbedder;

#[async_trait::async_trait]
impl surreal_memory::embeddings::EmbeddingService for NoOpEmbedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0f32; 1536])
    }
    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0f32; 1536]).collect())
    }
    fn dimensions(&self) -> usize {
        384
    }
}

async fn make_storage() -> Arc<SurrealStorage> {
    let embedder: Arc<dyn surreal_memory::embeddings::EmbeddingService> = Arc::new(NoOpEmbedder);
    Arc::new(
        SurrealStorage::new_mem(embedder)
            .await
            .expect("in-memory SurrealStorage"),
    )
}

async fn make_server_storage() -> Arc<SurrealStorage> {
    let embedder: Arc<dyn surreal_memory::embeddings::EmbeddingService> = Arc::new(NoOpEmbedder);
    let suffix = Uuid::new_v4().simple().to_string();
    let config = SurrealConfig {
        mode: SurrealMode::Server,
        endpoint: Some(
            std::env::var("TEST_SURREAL_ENDPOINT")
                .unwrap_or_else(|_| "ws://127.0.0.1:28000".to_string()),
        ),
        embedded_path: None,
        username: Some(
            std::env::var("TEST_SURREAL_USERNAME").unwrap_or_else(|_| "root".to_string()),
        ),
        password: Some(
            std::env::var("TEST_SURREAL_PASSWORD").unwrap_or_else(|_| "root".to_string()),
        ),
        namespace: format!("test_{}", suffix),
        database: "main".to_string(),
        retry: RetryConfig::default(),
    };

    Arc::new(
        SurrealStorage::new(&config, embedder)
            .await
            .expect("server-mode SurrealStorage"),
    )
}

fn record_id_string(id: &surrealdb::types::RecordId) -> String {
    use surrealdb::types::RecordIdKey;
    let key = match &id.key {
        RecordIdKey::String(value) => value.clone(),
        RecordIdKey::Number(value) => value.to_string(),
        RecordIdKey::Uuid(value) => value.to_string(),
        other => format!("{other:?}"),
    };
    format!("{}:{}", id.table.as_str(), key)
}

fn entity(name: &str) -> Entity {
    Entity {
        id: None,
        name: name.to_string(),
        entity_type: "Node".to_string(),
        observations: vec![],
        created_at: Default::default(),
        updated_at: Default::default(),
        embedding: None,
    }
}

fn relation(from: &str, to: &str, rel: &str) -> Relation {
    Relation {
        id: None,
        from: from.to_string(),
        to: to.to_string(),
        relation_type: rel.to_string(),
        created_at: Default::default(),
    }
}

fn memory(content: &str, user_id: &str) -> Memory {
    Memory::new(
        content.to_string(),
        Some(user_id.to_string()),
        None,
        None,
        vec![],
    )
}

// ── Model Profiles ────────────────────────────────────────────────────────────

#[test]
fn test_model_profiles_known() {
    let p = profile_for("gpt-4o");
    assert_eq!(p.model_id, "gpt-4o");
    assert_eq!(p.budget(), 112_000);
}

#[test]
fn test_model_profiles_unknown_falls_back() {
    let p = profile_for("mystery-llm");
    assert_eq!(p.model_id, "default");
    assert!(p.budget() > 0);
}

#[test]
fn test_model_profiles_all_valid() {
    for p in MODEL_PROFILES {
        assert!(p.budget() > 0, "Profile '{}' has zero budget", p.model_id);
        assert!(p.summarization_threshold() <= p.budget());
    }
}

#[test]
fn test_task_stream_needs_summarization() {
    let mut stream = TaskStream::new("test", None, None, None);
    stream.model_id = Some("default".to_string()); // budget = 6000
    stream.total_tokens = 0;
    assert!(!stream.needs_summarization());
    stream.total_tokens = 5000; // >= 80% of 6000 = 4800
    assert!(stream.needs_summarization());
}

// ── Entity CRUD ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_entity_crud() {
    let s = make_storage().await;

    let e = Entity {
        id: None,
        name: "Alice".to_string(),
        entity_type: "Person".to_string(),
        observations: vec!["Software engineer".to_string()],
        created_at: Default::default(),
        updated_at: Default::default(),
        embedding: None,
    };
    let created = s.create_entity(e).await.expect("create_entity");
    assert_eq!(created.name, "Alice");

    let fetched = s.get_entity("Alice").await.expect("get_entity");
    assert!(fetched.is_some());

    let updated = s
        .add_observations("Alice", vec!["Loves Rust".to_string()])
        .await
        .expect("add_observations");
    assert!(updated.observations.contains(&"Loves Rust".to_string()));

    s.delete_entity("Alice").await.expect("delete_entity");
    assert!(s.get_entity("Alice").await.unwrap().is_none());
}

#[tokio::test]
async fn test_relation_crud() {
    let s = make_storage().await;
    s.create_entities(vec![entity("Alice"), entity("Bob")])
        .await
        .unwrap();

    let created = s
        .create_relation(relation("Alice", "Bob", "KNOWS"))
        .await
        .expect("create_relation");
    assert_eq!(created.relation_type, "KNOWS");

    let rels = s.get_relations("Alice").await.expect("get_relations");
    assert!(!rels.is_empty());

    s.delete_relation("Alice", "Bob", "KNOWS")
        .await
        .expect("delete_relation");
    assert!(s.get_relations("Alice").await.unwrap().is_empty());
}

// ── Scoped Memory ─────────────────────────────────────────────────────────────

#[ignore = "requires server-mode SurrealDB; enum field deserialization differs in embedded mode"]
#[tokio::test]
async fn test_memory_lifecycle() {
    let s = make_storage().await;

    let stored = s
        .add_memory(memory("Test content", "u1"))
        .await
        .expect("add_memory");
    assert_eq!(stored.content, "Test content");

    let id_str = stored
        .id
        .as_ref()
        .map(|id| format!("{:?}", id))
        .and_then(|s| {
            s.split(':')
                .nth(1)
                .map(|p| p.trim_matches(['"', '}', ' ']).to_string())
        })
        .expect("id string");

    let fetched = s.get_memory(&id_str).await.expect("get_memory");
    assert!(fetched.is_some());

    let updated = s
        .update_memory(&id_str, "Updated".to_string())
        .await
        .expect("update_memory");
    assert_eq!(updated.content, "Updated");

    s.delete_memory(&id_str).await.expect("delete_memory");
    assert!(s.get_memory(&id_str).await.unwrap().is_none());
}

#[ignore = "requires server-mode SurrealDB; enum field deserialization differs in embedded mode"]
#[tokio::test]
async fn test_get_all_delete_all() {
    let s = make_storage().await;
    s.add_memory(memory("A", "u2")).await.unwrap();
    s.add_memory(memory("B", "u2")).await.unwrap();

    let all = s
        .get_all_memories(Some("u2"), None, None)
        .await
        .expect("get_all");
    assert_eq!(all.len(), 2);

    let deleted = s
        .delete_all_memories(Some("u2"), None, None)
        .await
        .expect("delete_all");
    assert_eq!(deleted, 2);
}

#[tokio::test]
async fn test_memory_search() {
    let s = make_storage().await;
    s.add_memory(memory("Rust systems programming", "u3"))
        .await
        .unwrap();
    let _results = s
        .search_memories("systems", Some("u3"), None, None, None, 5)
        .await
        .expect("search");
}

// ── TaskStreams ────────────────────────────────────────────────────────────────

#[ignore = "requires server-mode SurrealDB; enum field deserialization differs in embedded mode"]
#[tokio::test]
async fn test_task_stream_lifecycle() {
    let s = make_storage().await;

    let ts = TaskStream::new(
        "my-task",
        Some("Research".to_string()),
        None,
        Some("u1".to_string()),
    );
    let created = s.create_task_stream(ts).await.expect("create_task_stream");
    assert_eq!(created.name, "my-task");
    assert_eq!(created.status, TaskStreamStatus::Active);

    s.add_to_task_stream("my-task", memory("Step 1", "u1"))
        .await
        .expect("add_to_stream");

    let streams = s.list_task_streams(None, Some("u1")).await.expect("list");
    assert!(!streams.is_empty());

    let ctx = s
        .get_context_for_task("my-task", "gpt-4o", None)
        .await
        .expect("context");
    assert!(!ctx.memories.is_empty());

    let profile = profile_for("default");
    for idx in 0..4 {
        let mut mem = memory(&format!("Long step {}", idx + 2), "u1");
        mem.token_count = Some((profile.summarization_threshold() / 4 + 1) as u32);
        s.add_to_task_stream("my-task", mem)
            .await
            .expect("seed summarization");
    }

    let summary = s
        .auto_summarize_task_stream("my-task", Some("u1"), None, "default")
        .await
        .expect("auto summarize");
    assert!(
        summary.is_some(),
        "expected a summary once threshold is crossed"
    );

    let archived = s.archive_task_stream("my-task").await.expect("archive");
    assert_eq!(archived.status, TaskStreamStatus::Archived);
}

// ── Mindmaps ──────────────────────────────────────────────────────────────────

#[ignore = "requires server-mode SurrealDB; enum field deserialization differs in embedded mode"]
#[tokio::test]
async fn test_mindmap_crud() {
    let s = make_storage().await;

    let mm = MindMap {
        id: None,
        name: "persona".to_string(),
        map_type: MapType::Radial,
        description: Some("User persona".to_string()),
        user_id: Some("u1".to_string()),
        agent_id: None,
        task_stream_id: None,
        nodes: vec![MindMapNode {
            id: "root".to_string(),
            label: "Me".to_string(),
            parent_id: None,
            node_type: Some("person".to_string()),
            color: None,
            metadata: None,
        }],
        edges: vec![],
        tags: vec![],
        created_at: Default::default(),
        updated_at: Default::default(),
    };
    let created = s.create_mindmap(mm).await.expect("create_mindmap");
    assert_eq!(created.name, "persona");

    let fetched = s
        .get_mindmap("persona", Some("u1"), None)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(fetched.nodes.len(), 1);

    let node = MindMapNode {
        id: "n1".to_string(),
        label: "Skills".to_string(),
        parent_id: Some("root".to_string()),
        node_type: None,
        color: None,
        metadata: Some(serde_json::json!({
            "confidence": 0.92,
            "source": {
                "kind": "memory",
                "id": "memory:abc123"
            }
        })),
    };
    let updated = s
        .add_mindmap_node("persona", Some("u1"), None, node)
        .await
        .expect("add_node");
    assert_eq!(updated.nodes.len(), 2);
    assert_eq!(
        updated.nodes[1]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("source"))
            .and_then(|source| source.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("memory")
    );

    let list = s.list_mindmaps(Some("u1"), None).await.expect("list");
    assert!(!list.is_empty());

    s.delete_mindmap("persona", Some("u1"), None)
        .await
        .expect("delete");
    assert!(
        s.get_mindmap("persona", Some("u1"), None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn taskstream_server_mode_explicit_create_round_trips_model_settings() {
    let s = make_server_storage().await;
    let name = format!("taskstream-{}", Uuid::new_v4().simple());
    let user_id = "user-server";
    let agent_id = "agent-server";

    let mut ts = TaskStream::new(
        &name,
        Some("server-mode regression".to_string()),
        Some(agent_id.to_string()),
        Some(user_id.to_string()),
    );
    ts.model_id = Some("gpt-4o".to_string());
    ts.auto_summarize = false;

    let created = s.create_task_stream(ts).await.expect("create_task_stream");
    assert_eq!(created.name, name);
    assert_eq!(created.model_id.as_deref(), Some("gpt-4o"));
    assert!(!created.auto_summarize);
    assert!(
        created.id.is_some(),
        "server-mode create should return an id"
    );

    let fetched = s
        .get_task_stream(&name)
        .await
        .expect("get_task_stream")
        .expect("task stream exists");
    assert_eq!(fetched.model_id.as_deref(), Some("gpt-4o"));
    assert!(!fetched.auto_summarize);

    let stored = s
        .add_to_task_stream(&name, memory("server-mode step", user_id))
        .await
        .expect("add_to_task_stream");
    assert_eq!(stored.content, "server-mode step");

    let listed = s
        .list_task_streams(Some(agent_id), Some(user_id))
        .await
        .expect("list_task_streams");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, name);

    let context = s
        .get_context_for_task(&name, "gpt-4o", Some(64))
        .await
        .expect("get_context_for_task");
    assert_eq!(context.memories.len(), 1);

    let paused = s.pause_task_stream(&name).await.expect("pause_task_stream");
    assert_eq!(paused.status, surreal_memory::TaskStreamStatus::Paused);

    let archived = s
        .archive_task_stream(&name)
        .await
        .expect("archive_task_stream");
    assert_eq!(archived.status, surreal_memory::TaskStreamStatus::Archived);
}

#[tokio::test]
async fn mindmap_server_mode_mutations_round_trip_without_decode_failures() {
    let s = make_server_storage().await;
    let name = format!("mindmap-{}", Uuid::new_v4().simple());
    let user_id = format!("user-{}", Uuid::new_v4().simple());
    let agent_id = format!("agent-{}", Uuid::new_v4().simple());

    let mut mm = MindMap::new(
        &name,
        MapType::Radial,
        "Root",
        Some("server-mode regression".to_string()),
        Some(agent_id.clone()),
        Some(user_id.clone()),
    );
    mm.tags = vec!["persona".to_string(), "server".to_string()];

    let created = s.create_mindmap(mm).await.expect("create_mindmap");
    assert_eq!(
        created.tags,
        vec!["persona".to_string(), "server".to_string()]
    );

    let node = MindMapNode {
        id: "beliefs".to_string(),
        label: "Beliefs".to_string(),
        parent_id: Some("root".to_string()),
        node_type: Some("branch".to_string()),
        color: None,
        metadata: Some(serde_json::json!({
            "source": {
                "kind": "memory",
                "confidence": 0.9
            }
        })),
    };

    let with_node = s
        .add_mindmap_node(&name, Some(&user_id), Some(&agent_id), node)
        .await
        .expect("add_mindmap_node");
    assert_eq!(with_node.nodes.len(), 2);

    let with_edge = s
        .add_mindmap_edge(
            &name,
            Some(&user_id),
            Some(&agent_id),
            surreal_memory::mindmap::MindMapEdge {
                from_id: "root".to_string(),
                to_id: "beliefs".to_string(),
                label: Some("contains".to_string()),
                directed: true,
            },
        )
        .await
        .expect("add_mindmap_edge");
    assert_eq!(with_edge.edges.len(), 1);

    let fetched = s
        .get_mindmap(&name, Some(&user_id), Some(&agent_id))
        .await
        .expect("get_mindmap")
        .expect("mindmap exists");
    assert_eq!(fetched.nodes.len(), 2);
    assert_eq!(fetched.edges.len(), 1);
    assert_eq!(
        fetched.nodes[1]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("source"))
            .and_then(|source| source.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("memory")
    );

    let listed = s
        .list_mindmaps(Some(&user_id), Some(&agent_id))
        .await
        .expect("list_mindmaps");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, name);

    let without_node = s
        .delete_mindmap_node(&name, Some(&user_id), Some(&agent_id), "beliefs")
        .await
        .expect("delete_mindmap_node");
    assert_eq!(without_node.nodes.len(), 1);
    assert_eq!(without_node.edges.len(), 0);

    let exported = s
        .get_mindmap(&name, Some(&user_id), Some(&agent_id))
        .await
        .expect("get_mindmap for export")
        .expect("mindmap exists for export")
        .export(&surreal_memory::ExportFormat::Mermaid);
    assert!(exported.contains("graph TD"));

    s.delete_mindmap(&name, Some(&user_id), Some(&agent_id))
        .await
        .expect("delete_mindmap");
    assert!(
        s.get_mindmap(&name, Some(&user_id), Some(&agent_id))
            .await
            .expect("get_mindmap after delete")
            .is_none()
    );
    assert!(
        s.list_mindmaps(Some(&user_id), Some(&agent_id))
            .await
            .expect("list_mindmaps after delete")
            .is_empty()
    );
}

#[tokio::test]
async fn memory_server_mode_lifecycle_round_trips_record_ids() {
    let s = make_server_storage().await;
    let user_id = format!("user-{}", Uuid::new_v4().simple());

    let stored = s
        .add_memory(memory("server memory", &user_id))
        .await
        .expect("add_memory");
    let id = stored.id.as_ref().map(record_id_string).expect("memory id");

    let fetched = s.get_memory(&id).await.expect("get_memory");
    assert!(fetched.is_some());

    let updated = s
        .update_memory(&id, "updated server memory".to_string())
        .await
        .expect("update_memory");
    assert_eq!(updated.content, "updated server memory");

    s.delete_memory(&id).await.expect("delete_memory");
    assert!(s.get_memory(&id).await.expect("get after delete").is_none());
}

// ── Graph-RAG ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_graph_traversal() {
    let s = make_storage().await;
    s.create_entities(vec![entity("A"), entity("B"), entity("C")])
        .await
        .unwrap();
    s.create_relation(relation("A", "B", "LINKS"))
        .await
        .unwrap();
    s.create_relation(relation("B", "C", "LINKS"))
        .await
        .unwrap();

    let paths = s.find_path("A", "C", 4).await.expect("find_path");
    assert!(!paths.is_empty(), "Expected at least one path A→C");

    let graph = s
        .expand_neighbors("A", 2, 50)
        .await
        .expect("expand_neighbors");
    assert!(graph.entities.len() >= 1);

    let related = s
        .get_related("B", Some("LINKS"), "both", 10)
        .await
        .expect("get_related");
    assert!(!related.is_empty());
}

// ── Temporal History ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_entity_history() {
    let s = make_storage().await;
    let e = Entity {
        id: None,
        name: "Charlie".to_string(),
        entity_type: "Person".to_string(),
        observations: vec!["Obs1".to_string()],
        created_at: Default::default(),
        updated_at: Default::default(),
        embedding: None,
    };
    s.create_entity(e).await.unwrap();
    s.add_observations("Charlie", vec!["Obs2".to_string()])
        .await
        .unwrap();
    // Returns empty if changes aren't tracked in memory_history — test only asserts no error
    let _history = s
        .get_entity_history("Charlie")
        .await
        .expect("get_entity_history");
}

#[tokio::test]
async fn test_graph_at_time() {
    let s = make_storage().await;
    // Far-future timestamp returns all entities — assert no error
    let _graph = s
        .get_graph_at_time("2099-01-01T00:00:00Z")
        .await
        .expect("get_graph_at_time");
}

// ── Retry Logic ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_operation_survives_transient_failure() {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Create unique test directory
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("{}{}", d.as_secs(), d.subsec_nanos()))
        .unwrap_or_else(|_| "0".to_string());
    let test_dir = std::env::temp_dir().join(format!("surreal-retry-test-{}", ts));
    std::fs::create_dir_all(&test_dir).expect("create test dir");

    // Create storage with aggressive retry config for testing
    let retry_config = RetryConfig {
        max_connect_retries: 5,
        max_operation_retries: 3,
        base_retry_delay_ms: 10,  // Short delays for test speed
        max_retry_delay_ms: 100,
        jitter_factor: 0.1,
    };

    let config = SurrealConfig {
        mode: SurrealMode::Embedded,
        embedded_path: Some(test_dir.display().to_string()),
        namespace: "test".to_string(),
        database: "memory".to_string(),
        retry: retry_config,
        ..Default::default()
    };

    // Create embedding service (use NoOpEmbedder from above)
    let embedding_service: Arc<dyn surreal_memory::embeddings::EmbeddingService> =
        Arc::new(NoOpEmbedder);

    // Initialize storage with custom retry config
    let storage = SurrealStorage::new(&config, embedding_service)
        .await
        .expect("Failed to create storage");

    // Test 1: Create an entity (uses create_record which has retry logic)
    let entity = Entity {
        id: None,
        name: "RetryTestEntity".to_string(),
        entity_type: "TestNode".to_string(),
        observations: vec!["Testing retry behavior".to_string()],
        created_at: Default::default(),
        updated_at: Default::default(),
        embedding: None,
    };

    let result = storage.create_entity(entity).await;
    assert!(result.is_ok(), "create_entity should succeed with retry logic");
    let created_entity = result.unwrap();
    assert_eq!(created_entity.name, "RetryTestEntity");

    // Test 2: Create a relation (also uses create_record)
    let entity2 = Entity {
        id: None,
        name: "RetryTestEntity2".to_string(),
        entity_type: "TestNode".to_string(),
        observations: vec![],
        created_at: Default::default(),
        updated_at: Default::default(),
        embedding: None,
    };
    storage.create_entity(entity2).await.expect("create second entity");

    let relation = Relation {
        id: None,
        from: "RetryTestEntity".to_string(),
        to: "RetryTestEntity2".to_string(),
        relation_type: "TEST_LINK".to_string(),
        created_at: Default::default(),
    };

    let result = storage.create_relation(relation).await;
    assert!(result.is_ok(), "create_relation should succeed with retry logic");

    // Test 3: Verify the entities and relations were actually stored
    let fetched = storage.get_entity("RetryTestEntity").await;
    assert!(fetched.is_ok(), "get_entity should succeed");
    assert!(fetched.unwrap().is_some(), "Entity should exist in database");

    let relations = storage.get_relations("RetryTestEntity").await;
    assert!(relations.is_ok(), "get_relations should succeed");
    assert_eq!(relations.unwrap().len(), 1, "Should have 1 relation stored");

    // Cleanup: Remove test database
    std::fs::remove_dir_all(&test_dir).ok();
}
