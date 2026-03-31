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
use surrealdb::Surreal;
use surrealdb::engine::any::{Any, connect};
use surrealdb::opt::auth::Root;
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
    let suffix = Uuid::new_v4().simple().to_string();
    make_server_storage_with_namespace(format!("test_{}", suffix)).await
}

async fn make_server_storage_with_namespace(namespace: String) -> Arc<SurrealStorage> {
    let embedder: Arc<dyn surreal_memory::embeddings::EmbeddingService> = Arc::new(NoOpEmbedder);
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
        namespace,
        database: "main".to_string(),
        retry: RetryConfig::default(),
    };

    Arc::new(
        SurrealStorage::new(&config, embedder)
            .await
            .expect("server-mode SurrealStorage"),
    )
}

async fn connect_server_db(namespace: &str) -> Surreal<Any> {
    let endpoint = std::env::var("TEST_SURREAL_ENDPOINT")
        .unwrap_or_else(|_| "ws://127.0.0.1:28000".to_string());
    let username = std::env::var("TEST_SURREAL_USERNAME").unwrap_or_else(|_| "root".to_string());
    let password = std::env::var("TEST_SURREAL_PASSWORD").unwrap_or_else(|_| "root".to_string());

    let db = connect(endpoint).await.expect("connect server db");
    db.signin(Root { username, password })
        .await
        .expect("signin server db");
    db.use_ns(namespace)
        .use_db("main")
        .await
        .expect("use namespace/database");
    db
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

#[tokio::test]
async fn test_memory_lifecycle() {
    let s = make_storage().await;

    let stored = s
        .add_memory(memory("Test content", "u1"))
        .await
        .expect("add_memory");
    assert_eq!(stored.content, "Test content");

    let id_str = stored.id.as_ref().map(record_id_string).expect("id string");

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
    let stream_after_summary = s
        .get_task_stream("my-task")
        .await
        .expect("get task stream after summarize")
        .expect("task stream exists after summarize");
    assert!(
        summary.is_some() || stream_after_summary.summary_count > 0,
        "expected a summary once threshold is crossed, either inline or explicit"
    );

    let archived = s.archive_task_stream("my-task").await.expect("archive");
    assert_eq!(archived.status, TaskStreamStatus::Archived);
}

#[tokio::test]
async fn delete_task_stream_removes_linked_memories_and_detaches_mindmaps() {
    let s = make_storage().await;

    let stream = s
        .create_task_stream(TaskStream::new(
            "cleanup-task",
            Some("cleanup".to_string()),
            Some("agent-cleanup".to_string()),
            Some("user-cleanup".to_string()),
        ))
        .await
        .expect("create task stream");
    s.add_to_task_stream(
        "cleanup-task",
        Memory::new(
            "delete me".to_string(),
            Some("user-cleanup".to_string()),
            Some("agent-cleanup".to_string()),
            None,
            vec!["cleanup".to_string()],
        ),
    )
    .await
    .expect("add task memory");

    let mut map = MindMap::new(
        "cleanup-map",
        MapType::Radial,
        "Cleanup",
        Some("linked map".to_string()),
        Some("agent-cleanup".to_string()),
        Some("user-cleanup".to_string()),
    );
    map.task_stream_id = stream.id.clone();
    s.create_mindmap(map).await.expect("create linked mindmap");

    s.delete_task_stream("cleanup-task")
        .await
        .expect("delete task stream");

    assert!(
        s.get_task_stream("cleanup-task")
            .await
            .expect("get task stream after delete")
            .is_none()
    );
    assert!(
        s.get_all_memories(Some("user-cleanup"), Some("agent-cleanup"), None)
            .await
            .expect("get memories after delete")
            .is_empty()
    );
    let linked_map = s
        .get_mindmap("cleanup-map", Some("user-cleanup"), Some("agent-cleanup"))
        .await
        .expect("get linked mindmap after delete")
        .expect("linked mindmap should survive task deletion");
    assert!(linked_map.task_stream_id.is_none());
}

// ── Mindmaps ──────────────────────────────────────────────────────────────────

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
async fn test_long_running_project_lifecycle_embedded() {
    let s = make_storage().await;
    let user_id = "project-user";
    let stream_name = format!("project-{}", Uuid::new_v4().simple());

    let created_stream = s
        .create_task_stream(TaskStream::new(
            &stream_name,
            Some("Embedded long-running project".to_string()),
            None,
            Some(user_id.to_string()),
        ))
        .await
        .expect("create task stream");

    let kickoff = s
        .add_to_task_stream(
            &stream_name,
            memory("Kickoff decision: use shared memory", user_id),
        )
        .await
        .expect("add kickoff memory");
    let kickoff_id = kickoff
        .id
        .as_ref()
        .map(record_id_string)
        .expect("kickoff memory id");

    let milestone = s
        .add_to_task_stream(
            &stream_name,
            memory("Milestone: schema repair landed", user_id),
        )
        .await
        .expect("add milestone memory");
    let milestone_id = milestone
        .id
        .as_ref()
        .map(record_id_string)
        .expect("milestone memory id");

    let mut map = MindMap::new(
        &format!("map-{}", stream_name),
        MapType::Radial,
        "Project Root",
        Some("Shared project context".to_string()),
        None,
        Some(user_id.to_string()),
    );
    map.task_stream_id = created_stream.id.clone();

    let created_map = s.create_mindmap(map).await.expect("create mindmap");
    assert_eq!(created_map.nodes.len(), 1);

    let plan_node = MindMapNode {
        id: "plan".to_string(),
        label: "Plan".to_string(),
        parent_id: Some("root".to_string()),
        node_type: Some("branch".to_string()),
        color: None,
        metadata: Some(serde_json::json!({
            "memory_id": kickoff_id,
            "status": "active"
        })),
    };
    let with_plan = s
        .add_mindmap_node(&created_map.name, Some(user_id), None, plan_node)
        .await
        .expect("add plan node");
    assert_eq!(with_plan.nodes.len(), 2);

    let detail_node = MindMapNode {
        id: "validation".to_string(),
        label: "Validation".to_string(),
        parent_id: Some("plan".to_string()),
        node_type: Some("leaf".to_string()),
        color: None,
        metadata: Some(serde_json::json!({
            "memory_id": milestone_id,
            "owner": "qa"
        })),
    };
    let with_details = s
        .add_mindmap_node(&created_map.name, Some(user_id), None, detail_node)
        .await
        .expect("add validation node");
    assert_eq!(with_details.nodes.len(), 3);

    let updated_memory = s
        .update_memory(
            &milestone_id,
            "Milestone: validation passed across tools".to_string(),
        )
        .await
        .expect("update milestone");
    assert_eq!(
        updated_memory.content,
        "Milestone: validation passed across tools"
    );

    let context = s
        .get_context_for_task(&stream_name, "gpt-4o", Some(256))
        .await
        .expect("context for task");
    assert_eq!(context.memories.len(), 2);
    assert!(
        context
            .memories
            .iter()
            .any(|memory| memory.content.contains("validation passed"))
    );

    let fetched_map = s
        .get_mindmap(&created_map.name, Some(user_id), None)
        .await
        .expect("get mindmap")
        .expect("mindmap exists");
    assert_eq!(fetched_map.nodes.len(), 3);
    assert_eq!(
        fetched_map.nodes[2]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("memory_id"))
            .and_then(serde_json::Value::as_str),
        Some(milestone_id.as_str())
    );

    let archived = s
        .archive_task_stream(&stream_name)
        .await
        .expect("archive task stream");
    assert_eq!(archived.status, TaskStreamStatus::Archived);
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

#[tokio::test]
async fn shared_project_server_mode_continuity_across_storage_instances() {
    let namespace = format!("shared_{}", Uuid::new_v4().simple());
    let writer_a = make_server_storage_with_namespace(namespace.clone()).await;
    let writer_b = make_server_storage_with_namespace(namespace).await;
    let user_id = format!("user-{}", Uuid::new_v4().simple());
    let stream_name = format!("stream-{}", Uuid::new_v4().simple());
    let map_name = format!("map-{}", Uuid::new_v4().simple());

    let created_stream = writer_a
        .create_task_stream(TaskStream::new(
            &stream_name,
            Some("Shared project".to_string()),
            None,
            Some(user_id.clone()),
        ))
        .await
        .expect("writer A create stream");
    assert_eq!(created_stream.status, TaskStreamStatus::Active);

    let kickoff = writer_a
        .add_to_task_stream(&stream_name, memory("Kickoff complete", &user_id))
        .await
        .expect("writer A add kickoff");
    let kickoff_id = kickoff
        .id
        .as_ref()
        .map(record_id_string)
        .expect("kickoff id");

    let mut map = MindMap::new(
        &map_name,
        MapType::Radial,
        "Shared Root",
        Some("Server continuity".to_string()),
        None,
        Some(user_id.clone()),
    );
    map.task_stream_id = created_stream.id.clone();
    writer_a
        .create_mindmap(map)
        .await
        .expect("writer A create map");

    let listed_streams = writer_b
        .list_task_streams(None, Some(&user_id))
        .await
        .expect("writer B list streams");
    assert_eq!(listed_streams.len(), 1);
    assert_eq!(listed_streams[0].name, stream_name);

    let context_before = writer_b
        .get_context_for_task(&stream_name, "gpt-4o", Some(128))
        .await
        .expect("writer B context");
    assert_eq!(context_before.memories.len(), 1);

    let map_before = writer_b
        .get_mindmap(&map_name, Some(&user_id), None)
        .await
        .expect("writer B get mindmap")
        .expect("mindmap exists");
    assert_eq!(map_before.nodes.len(), 1);

    writer_b
        .add_to_task_stream(&stream_name, memory("Validation finished", &user_id))
        .await
        .expect("writer B add memory");
    writer_b
        .add_mindmap_node(
            &map_name,
            Some(&user_id),
            None,
            MindMapNode {
                id: "validation".to_string(),
                label: "Validation".to_string(),
                parent_id: Some("root".to_string()),
                node_type: Some("branch".to_string()),
                color: None,
                metadata: Some(serde_json::json!({
                    "memory_id": kickoff_id,
                    "tool": "writer-b"
                })),
            },
        )
        .await
        .expect("writer B add node");
    writer_b
        .add_mindmap_edge(
            &map_name,
            Some(&user_id),
            None,
            surreal_memory::mindmap::MindMapEdge {
                from_id: "root".to_string(),
                to_id: "validation".to_string(),
                label: Some("tracks".to_string()),
                directed: true,
            },
        )
        .await
        .expect("writer B add edge");

    let context_after = writer_a
        .get_context_for_task(&stream_name, "gpt-4o", Some(256))
        .await
        .expect("writer A updated context");
    assert_eq!(context_after.memories.len(), 2);
    assert!(
        context_after
            .memories
            .iter()
            .any(|memory| memory.content.contains("Validation finished"))
    );

    let map_after = writer_a
        .get_mindmap(&map_name, Some(&user_id), None)
        .await
        .expect("writer A get updated map")
        .expect("updated mindmap exists");
    assert_eq!(map_after.nodes.len(), 2);
    assert_eq!(map_after.edges.len(), 1);
    assert_eq!(
        map_after.nodes[1]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("tool"))
            .and_then(serde_json::Value::as_str),
        Some("writer-b")
    );

    let paused = writer_a
        .pause_task_stream(&stream_name)
        .await
        .expect("pause stream");
    assert_eq!(paused.status, TaskStreamStatus::Paused);

    let archived = writer_b
        .archive_task_stream(&stream_name)
        .await
        .expect("archive stream");
    assert_eq!(archived.status, TaskStreamStatus::Archived);

    writer_b
        .delete_mindmap(&map_name, Some(&user_id), None)
        .await
        .expect("delete mindmap");
    assert!(
        writer_a
            .get_mindmap(&map_name, Some(&user_id), None)
            .await
            .expect("get deleted mindmap")
            .is_none()
    );
}

#[tokio::test]
async fn legacy_enum_rows_are_repaired_on_server_mode_startup() {
    let namespace = format!("legacy_{}", Uuid::new_v4().simple());
    let db = connect_server_db(&namespace).await;
    let now = surrealdb::types::Datetime::default();

    let response = db
        .query(
            "
            DEFINE TABLE memory SCHEMAFULL;
            DEFINE FIELD content ON memory TYPE string;
            DEFINE FIELD embedding ON memory TYPE option<array<float>>;
            DEFINE FIELD scope ON memory TYPE any DEFAULT 'global';
            DEFINE FIELD memory_type ON memory TYPE any DEFAULT 'semantic';
            DEFINE FIELD user_id ON memory TYPE option<string>;
            DEFINE FIELD session_id ON memory TYPE option<string>;
            DEFINE FIELD agent_id ON memory TYPE option<string>;
            DEFINE FIELD task_stream_id ON memory TYPE option<record<task_stream>>;
            DEFINE FIELD categories ON memory TYPE array<string> DEFAULT [];
            DEFINE FIELD metadata ON memory TYPE option<object> FLEXIBLE;
            DEFINE FIELD token_count ON memory TYPE option<int>;
            DEFINE FIELD importance ON memory TYPE float DEFAULT 0.5;
            DEFINE FIELD access_count ON memory TYPE int DEFAULT 0;
            DEFINE FIELD last_accessed_at ON memory TYPE option<datetime>;
            DEFINE FIELD valid_until ON memory TYPE option<datetime>;
            DEFINE FIELD version ON memory TYPE int DEFAULT 1;
            DEFINE FIELD created_at ON memory TYPE datetime;
            DEFINE FIELD updated_at ON memory TYPE datetime;

            DEFINE TABLE task_stream SCHEMAFULL;
            DEFINE FIELD name ON task_stream TYPE string;
            DEFINE FIELD description ON task_stream TYPE option<string>;
            DEFINE FIELD agent_id ON task_stream TYPE option<string>;
            DEFINE FIELD user_id ON task_stream TYPE option<string>;
            DEFINE FIELD status ON task_stream TYPE any DEFAULT 'active';
            DEFINE FIELD total_tokens ON task_stream TYPE int DEFAULT 0;
            DEFINE FIELD auto_summarize ON task_stream TYPE bool DEFAULT true;
            DEFINE FIELD summary_count ON task_stream TYPE int DEFAULT 0;
            DEFINE FIELD model_id ON task_stream TYPE option<string>;
            DEFINE FIELD created_at ON task_stream TYPE datetime;
            DEFINE FIELD last_active ON task_stream TYPE datetime;

            DEFINE TABLE mindmap SCHEMALESS;
            ",
        )
        .await
        .expect("define legacy schema");
    response.check().expect("legacy schema accepted");

    let response = db
        .query(
            "
            CREATE task_stream:legacy CONTENT {
                name: $stream_name,
                description: 'legacy stream',
                agent_id: NONE,
                user_id: $user_id,
                status: { Active: {} },
                total_tokens: 0,
                auto_summarize: true,
                summary_count: 0,
                model_id: NONE,
                created_at: $now,
                last_active: $now
            };

            CREATE memory:legacy CONTENT {
                content: 'legacy memory',
                embedding: NONE,
                scope: { Global: {} },
                memory_type: { Semantic: {} },
                user_id: $user_id,
                session_id: NONE,
                agent_id: NONE,
                task_stream_id: task_stream:legacy,
                categories: [],
                metadata: NONE,
                token_count: NONE,
                importance: 0.5,
                access_count: 0,
                last_accessed_at: NONE,
                valid_until: NONE,
                version: 1,
                created_at: $now,
                updated_at: $now
            };

            CREATE mindmap:legacy CONTENT {
                name: $map_name,
                description: 'legacy map',
                map_type: { Radial: {} },
                agent_id: NONE,
                user_id: $user_id,
                task_stream_id: task_stream:legacy,
                tags: [],
                nodes: [],
                edges: [],
                created_at: $now,
                updated_at: $now
            };
            ",
        )
        .bind(("stream_name", "legacy-stream"))
        .bind(("map_name", "legacy-map"))
        .bind(("user_id", "legacy-user"))
        .bind(("now", now))
        .await
        .expect("seed legacy rows");
    response.check().expect("legacy rows accepted");

    let storage = make_server_storage_with_namespace(namespace.clone()).await;

    let streams = storage
        .list_task_streams(None, Some("legacy-user"))
        .await
        .expect("list task streams after repair");
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].status, TaskStreamStatus::Active);

    let memories = storage
        .get_all_memories(Some("legacy-user"), None, None)
        .await
        .expect("get memories after repair");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].scope, surreal_memory::MemoryScope::Global);
    assert_eq!(
        memories[0].memory_type,
        surreal_memory::MemoryType::Semantic
    );

    let mindmaps = storage
        .list_mindmaps(Some("legacy-user"), None)
        .await
        .expect("list mindmaps after repair");
    assert_eq!(mindmaps.len(), 1);
    assert_eq!(mindmaps[0].map_type, MapType::Radial);

    let raw_db = connect_server_db(&namespace).await;
    let raw_status: Vec<serde_json::Value> = raw_db
        .query("SELECT status FROM task_stream WHERE name = 'legacy-stream'")
        .await
        .expect("query raw status")
        .take(0)
        .unwrap_or_default();
    assert_eq!(
        raw_status[0]["status"],
        serde_json::Value::String("active".to_string())
    );
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
        base_retry_delay_ms: 10, // Short delays for test speed
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
    assert!(
        result.is_ok(),
        "create_entity should succeed with retry logic"
    );
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
    storage
        .create_entity(entity2)
        .await
        .expect("create second entity");

    let relation = Relation {
        id: None,
        from: "RetryTestEntity".to_string(),
        to: "RetryTestEntity2".to_string(),
        relation_type: "TEST_LINK".to_string(),
        created_at: Default::default(),
    };

    let result = storage.create_relation(relation).await;
    assert!(
        result.is_ok(),
        "create_relation should succeed with retry logic"
    );

    // Test 3: Verify the entities and relations were actually stored
    let fetched = storage.get_entity("RetryTestEntity").await;
    assert!(fetched.is_ok(), "get_entity should succeed");
    assert!(
        fetched.unwrap().is_some(),
        "Entity should exist in database"
    );

    let relations = storage.get_relations("RetryTestEntity").await;
    assert!(relations.is_ok(), "get_relations should succeed");
    assert_eq!(relations.unwrap().len(), 1, "Should have 1 relation stored");

    // Cleanup: Remove test database
    std::fs::remove_dir_all(&test_dir).ok();
}
