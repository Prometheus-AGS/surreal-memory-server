use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rmcp::{ErrorData as McpError, model::ErrorCode};
use serde_json::Value;
use surreal_memory::{
    MemoryStorage, embeddings::EmbeddingService, storage::surreal::SurrealStorage,
};
use surreal_memory_server::{
    api,
    contracts::{
        AddMemoryRequest, AddMindmapNodeRequest, CreateEntityRequest, CreateMindmapRequest,
    },
    mcp::handlers::{AddMindmapNodeParams, CreateEntitiesParams, MemoryHandler},
};
use tower::ServiceExt;

struct NoOpEmbedder;

#[async_trait::async_trait]
impl EmbeddingService for NoOpEmbedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0; 1536])
    }

    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.into_iter().map(|_| vec![0.0; 1536]).collect())
    }

    fn dimensions(&self) -> usize {
        1536
    }
}

async fn make_storage() -> Arc<dyn MemoryStorage> {
    let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
    Arc::new(
        SurrealStorage::new_mem(embedder)
            .await
            .expect("in-memory SurrealStorage"),
    )
}

fn mcp_code(error: &McpError) -> ErrorCode {
    error.code
}

#[tokio::test]
async fn mcp_and_rest_reject_invalid_mindmap_map_type() {
    let storage = make_storage().await;
    let router = api::build_router(Arc::clone(&storage));
    let body = CreateMindmapRequest {
        name: "bad-map-type".to_string(),
        map_type: Some("not-a-map-type".to_string()),
        root_label: "Root".to_string(),
        description: None,
        agent_id: None,
        user_id: Some("contract-user".to_string()),
        task_stream_id: None,
        tags: None,
    };

    let rest_response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mindmaps")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("json body")))
                .expect("request"),
        )
        .await
        .expect("rest response");
    assert_eq!(rest_response.status(), StatusCode::BAD_REQUEST);

    let handler = MemoryHandler::new(storage);
    let mcp_error = handler
        .create_mindmap(body.into())
        .await
        .expect_err("invalid map type should fail through MCP");
    assert_eq!(mcp_code(&mcp_error), ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn mcp_batch_create_entities_validates_every_item() {
    let storage = make_storage().await;
    let handler = MemoryHandler::new(storage);

    let mcp_error = handler
        .create_entities(CreateEntitiesParams {
            entities: vec![
                CreateEntityRequest {
                    name: "Valid entity".to_string(),
                    entity_type: "Concept".to_string(),
                    observations: vec!["Has one useful observation".to_string()],
                }
                .into(),
                CreateEntityRequest {
                    name: "Invalid entity".to_string(),
                    entity_type: "Concept".to_string(),
                    observations: Vec::new(),
                }
                .into(),
            ],
        })
        .await
        .expect_err("invalid batch item should fail before storage");

    assert_eq!(mcp_code(&mcp_error), ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn rest_and_mcp_add_memory_persist_same_contract_fields() {
    let rest_storage = make_storage().await;
    let rest_router = api::build_router(Arc::clone(&rest_storage));
    let mcp_storage = make_storage().await;
    let mcp_handler = MemoryHandler::new(Arc::clone(&mcp_storage));

    let body = AddMemoryRequest {
        content: "contract parity memory".to_string(),
        user_id: Some("contract-user".to_string()),
        agent_id: Some("contract-agent".to_string()),
        session_id: Some("contract-session".to_string()),
        categories: Some(vec!["contract".to_string(), "parity".to_string()]),
    };

    let rest_response = rest_router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/memory")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("json body")))
                .expect("request"),
        )
        .await
        .expect("rest response");
    assert_eq!(rest_response.status(), StatusCode::CREATED);

    mcp_handler
        .add_memory(body.clone().into())
        .await
        .expect("mcp add_memory");

    let rest_memories = rest_storage
        .get_all_memories(
            body.user_id.as_deref(),
            body.agent_id.as_deref(),
            body.session_id.as_deref(),
        )
        .await
        .expect("rest memories");
    let mcp_memories = mcp_storage
        .get_all_memories(
            body.user_id.as_deref(),
            body.agent_id.as_deref(),
            body.session_id.as_deref(),
        )
        .await
        .expect("mcp memories");

    let rest_memory = rest_memories.first().expect("rest memory");
    let mcp_memory = mcp_memories.first().expect("mcp memory");
    assert_eq!(rest_memory.content, mcp_memory.content);
    assert_eq!(rest_memory.user_id, mcp_memory.user_id);
    assert_eq!(rest_memory.agent_id, mcp_memory.agent_id);
    assert_eq!(rest_memory.session_id, mcp_memory.session_id);
    assert_eq!(rest_memory.categories, mcp_memory.categories);
}

#[tokio::test]
async fn rest_and_mcp_create_entity_persist_same_contract_fields() {
    let rest_storage = make_storage().await;
    let rest_router = api::build_router(Arc::clone(&rest_storage));
    let mcp_storage = make_storage().await;
    let mcp_handler = MemoryHandler::new(Arc::clone(&mcp_storage));

    let body = CreateEntityRequest {
        name: "Contract Entity".to_string(),
        entity_type: "Concept".to_string(),
        observations: vec![
            "Created through both REST and MCP".to_string(),
            "Used for contract parity checks".to_string(),
        ],
    };

    let rest_response = rest_router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/entities")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("json body")))
                .expect("request"),
        )
        .await
        .expect("rest response");
    assert_eq!(rest_response.status(), StatusCode::CREATED);

    mcp_handler
        .create_entity(body.clone().into())
        .await
        .expect("mcp create_entity");

    let rest_entity = rest_storage
        .get_entity(&body.name)
        .await
        .expect("rest entity")
        .expect("rest entity exists");
    let mcp_entity = mcp_storage
        .get_entity(&body.name)
        .await
        .expect("mcp entity")
        .expect("mcp entity exists");

    assert_eq!(rest_entity.name, mcp_entity.name);
    assert_eq!(rest_entity.entity_type, mcp_entity.entity_type);
    assert_eq!(rest_entity.observations, mcp_entity.observations);
}

#[tokio::test]
async fn rest_and_mcp_mindmap_nodes_preserve_nested_metadata() {
    let rest_storage = make_storage().await;
    let rest_router = api::build_router(Arc::clone(&rest_storage));
    let mcp_storage = make_storage().await;
    let mcp_handler = MemoryHandler::new(Arc::clone(&mcp_storage));

    let create_body = CreateMindmapRequest {
        name: "metadata-map".to_string(),
        map_type: Some("radial".to_string()),
        root_label: "Root".to_string(),
        description: None,
        agent_id: None,
        user_id: Some("metadata-user".to_string()),
        task_stream_id: None,
        tags: None,
    };
    let node_body = AddMindmapNodeRequest {
        node_id: "nested".to_string(),
        label: "Nested Metadata".to_string(),
        parent_id: Some("root".to_string()),
        node_type: Some("branch".to_string()),
        color: None,
        metadata: Some(serde_json::json!({
            "source": {
                "kind": "memory",
                "confidence": 0.97
            }
        })),
    };

    let create_response = rest_router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mindmaps")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&create_body).expect("json body"),
                ))
                .expect("request"),
        )
        .await
        .expect("rest create response");
    assert_eq!(create_response.status(), StatusCode::CREATED);

    let add_node_response = rest_router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mindmaps/metadata-map/nodes?user_id=metadata-user")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&node_body).expect("json body"),
                ))
                .expect("request"),
        )
        .await
        .expect("rest add node response");
    assert_eq!(add_node_response.status(), StatusCode::OK);

    mcp_handler
        .create_mindmap(create_body.clone().into())
        .await
        .expect("mcp create_mindmap");
    mcp_handler
        .add_mindmap_node(AddMindmapNodeParams {
            mindmap_name: create_body.name.clone(),
            user_id: create_body.user_id.clone(),
            agent_id: create_body.agent_id.clone(),
            node_id: node_body.node_id.clone(),
            label: node_body.label.clone(),
            parent_id: node_body.parent_id.clone(),
            node_type: node_body.node_type.clone(),
            color: node_body.color.clone(),
            metadata: node_body.metadata.clone(),
        })
        .await
        .expect("mcp add_mindmap_node");

    let rest_map = rest_storage
        .get_mindmap("metadata-map", Some("metadata-user"), None)
        .await
        .expect("rest mindmap")
        .expect("rest mindmap exists");
    let mcp_map = mcp_storage
        .get_mindmap("metadata-map", Some("metadata-user"), None)
        .await
        .expect("mcp mindmap")
        .expect("mcp mindmap exists");

    assert_eq!(rest_map.nodes[1].metadata, node_body.metadata);
    assert_eq!(mcp_map.nodes[1].metadata, node_body.metadata);
}

#[tokio::test]
async fn mcp_and_rest_reject_invalid_mindmap_task_stream_id() {
    let storage = make_storage().await;
    let router = api::build_router(Arc::clone(&storage));
    let body = CreateMindmapRequest {
        name: "bad-task-stream-id".to_string(),
        map_type: Some("radial".to_string()),
        root_label: "Root".to_string(),
        description: None,
        agent_id: None,
        user_id: None,
        task_stream_id: Some("not-a-record-id".to_string()),
        tags: None,
    };

    let rest_response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mindmaps")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("json body")))
                .expect("request"),
        )
        .await
        .expect("rest response");
    assert_eq!(rest_response.status(), StatusCode::BAD_REQUEST);

    let handler = MemoryHandler::new(storage);
    let mcp_error = handler
        .create_mindmap(body.into())
        .await
        .expect_err("invalid task stream id should fail through MCP");
    assert_eq!(mcp_code(&mcp_error), ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn committed_contract_specs_match_generated_contracts() {
    let expected_mcp: Value = serde_json::from_str(include_str!("../docs/specs/mcp-tools.json"))
        .expect("mcp-tools.json should be valid JSON");
    let expected_rest: Value = serde_json::from_str(include_str!("../docs/specs/rest-api.json"))
        .expect("rest-api.json should be valid JSON");

    assert_eq!(
        expected_mcp,
        surreal_memory_server::contracts::mcp_tools_spec()
    );
    assert_eq!(
        expected_rest,
        surreal_memory_server::contracts::rest_api_spec()
    );
}
