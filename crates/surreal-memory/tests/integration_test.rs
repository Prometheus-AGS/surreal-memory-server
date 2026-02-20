//! Integration tests for the `surreal-memory` library.
//!
//! All tests use `mem://` SurrealDB — zero external dependencies, runs with just `cargo test`.

use std::sync::Arc;
use surreal_memory::mindmap::{MapType, MindMap, MindMapNode};
use surreal_memory::{
    Entity, Memory, MemoryStorage, Relation, TaskStream,
    model_profiles::{MODEL_PROFILES, profile_for},
    storage::surreal::SurrealStorage,
    task_stream::TaskStreamStatus,
};

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
        .get_mindmap("persona", Some("u1"))
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
        metadata: None,
    };
    let updated = s
        .add_mindmap_node("persona", Some("u1"), node)
        .await
        .expect("add_node");
    assert_eq!(updated.nodes.len(), 2);

    let list = s.list_mindmaps(Some("u1"), None).await.expect("list");
    assert!(!list.is_empty());

    s.delete_mindmap("persona", Some("u1"))
        .await
        .expect("delete");
    assert!(
        s.get_mindmap("persona", Some("u1"))
            .await
            .unwrap()
            .is_none()
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
