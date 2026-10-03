use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use surreal_memory::{
    EmbeddingService, MemoryStorage, RetryConfig, SurrealConfig, SurrealStorage,
    storage::surreal::SurrealMode,
};
use surreal_memory_server::api;
use tower::ServiceExt;

struct NoOpEmbedder;

struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn start_server() -> (ServerGuard, String) {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let child = Command::new("surreal")
        .args([
            "start",
            "--no-banner",
            "--unauthenticated",
            "--allow-all",
            "--bind",
            &address.to_string(),
            "memory",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("installed surreal CLI starts the isolated fixture");
    let guard = ServerGuard(child);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("isolated surreal fixture becomes reachable");
    (guard, format!("ws://{address}"))
}

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

#[tokio::test]
async fn concurrent_receipt_timeouts_leave_the_same_coordinator_able_to_commit_later_work() {
    let (_server, endpoint) = start_server().await;
    let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
    let storage = Arc::new(
        SurrealStorage::new(
            &SurrealConfig {
                auth_level: Default::default(),
                mode: SurrealMode::Server,
                endpoint: Some(endpoint),
                embedded_path: None,
                username: None,
                password: None,
                namespace: format!("deadline_{}", uuid::Uuid::new_v4().simple()),
                database: "operations".to_owned(),
                retry: RetryConfig {
                    max_connect_retries: 0,
                    query_timeout_ms: 10_000,
                    ..RetryConfig::default()
                },
            },
            Arc::clone(&embedder),
        )
        .await
        .expect("isolated server-mode SurrealStorage"),
    );
    let database = storage.db().unwrap().clone();
    let large_receipt_key = Sha256::digest(b"large-receipt")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let empty_payload = json!({});
    let empty_payload_hash = Sha256::digest(serde_json::to_vec(&empty_payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    database
        .query(
            "CREATE type::record('memory_operation', $key) CONTENT {
                operation_id: $id,
                schema_version: 2,
                kind: 'create_task_stream',
                dependencies: [],
                payload_hash: $payload_hash,
                payload: $payload,
                state: 'committed',
                blocked_by: [],
                result: $result,
                error: NONE,
                executor_generation: 0,
                executor_progress_seq: 0,
                executor_exit_count: 0,
                executor_last_exit: NONE,
                executor_error: NONE,
                progress_seq: 1,
                created_at: time::now(),
                updated_at: time::now()
            }",
        )
        .bind(("key", large_receipt_key))
        .bind(("id", "large-receipt".to_owned()))
        .bind(("payload_hash", empty_payload_hash))
        .bind(("payload", empty_payload))
        .bind(("result", json!({"blob":"x".repeat(32 * 1024 * 1024)})))
        .await
        .unwrap()
        .check()
        .unwrap();
    let router = api::build_router_with_query_timeout(
        Arc::clone(&storage) as Arc<dyn MemoryStorage>,
        embedder,
        Duration::from_millis(10),
    );

    let timeout_batch = futures_util::future::join_all((0..4).map(|_| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .uri("/api/v2/operations/large-receipt")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    }));
    let general_storage_health = async {
        tokio::time::sleep(Duration::from_millis(1)).await;
        tokio::time::timeout(Duration::from_millis(500), storage.health_check())
            .await
            .expect("general storage remains responsive during ledger cancellation")
            .expect("general storage health query succeeds")
    };
    let (timeout_responses, storage_healthy) = tokio::join!(timeout_batch, general_storage_health);
    assert!(storage_healthy);
    for timeout_response in timeout_responses {
        assert_eq!(timeout_response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let timeout_body: Value = serde_json::from_slice(
            &to_bytes(timeout_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            timeout_body["error"],
            "operation database receipt lookup timed out after 10ms"
        );
    }

    let payload = json!({
        "name": "deadline-probe",
        "description": "proves coordinator recovery after query cancellation",
        "agent_id": null,
        "user_id": "test"
    });
    let payload_hash = Sha256::digest(serde_json::to_vec(&payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let request_body = json!({
        "operation_id": "deadline-probe",
        "schema_version": 2,
        "kind": "create_task_stream",
        "dependencies": [],
        "payload_hash": payload_hash,
        "payload": payload
    });

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/operations")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let submit_status = response.status();
    let submit_body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        submit_status,
        StatusCode::ACCEPTED,
        "submit response body: {}",
        String::from_utf8_lossy(&submit_body)
    );

    let receipt = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/v2/operations/deadline-probe")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(
                status,
                StatusCode::OK,
                "receipt response body: {}",
                String::from_utf8_lossy(&bytes)
            );
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            if body["state"] == "committed" {
                break body;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the original coordinator commits later work");
    assert_eq!(receipt["operation_id"], "deadline-probe");
}

#[tokio::test]
async fn startup_reconciliation_processes_a_dependency_in_a_later_drain_wave() {
    let (_server, endpoint) = start_server().await;
    let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
    let storage = Arc::new(
        SurrealStorage::new(
            &SurrealConfig {
                auth_level: Default::default(),
                mode: SurrealMode::Server,
                endpoint: Some(endpoint),
                embedded_path: None,
                username: None,
                password: None,
                namespace: format!("reconcile_{}", uuid::Uuid::new_v4().simple()),
                database: "operations".to_owned(),
                retry: RetryConfig {
                    max_connect_retries: 0,
                    query_timeout_ms: 10_000,
                    ..RetryConfig::default()
                },
            },
            Arc::clone(&embedder),
        )
        .await
        .expect("isolated server-mode SurrealStorage"),
    );
    let database = storage.db().unwrap().clone();
    let dependent_payload = json!({
        "name": "dependent",
        "description": "must wait for the later-sorted prerequisite",
        "agent_id": null,
        "user_id": "test"
    });
    let prerequisite_payload = json!({
        "name": "prerequisite",
        "description": "commits in the first drain wave",
        "agent_id": null,
        "user_id": "test"
    });
    let dependent_hash = Sha256::digest(serde_json::to_vec(&dependent_payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let prerequisite_hash = Sha256::digest(serde_json::to_vec(&prerequisite_payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let dependent_key = Sha256::digest(b"a-dependent")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let prerequisite_key = Sha256::digest(b"z-prerequisite")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    for (key, operation_id, dependencies, payload_hash, payload) in [
        (
            dependent_key,
            "a-dependent",
            vec!["z-prerequisite"],
            dependent_hash,
            dependent_payload,
        ),
        (
            prerequisite_key,
            "z-prerequisite",
            Vec::new(),
            prerequisite_hash,
            prerequisite_payload,
        ),
    ] {
        database
            .query(
                "CREATE type::record('memory_operation', $key) CONTENT {
                    operation_id: $id,
                    schema_version: 2,
                    kind: 'create_task_stream',
                    dependencies: $dependencies,
                    payload_hash: $payload_hash,
                    payload: $payload,
                    state: 'accepted',
                    blocked_by: [],
                    result: NONE,
                    error: NONE,
                    executor_generation: 0,
                    executor_progress_seq: 0,
                    executor_exit_count: 0,
                    executor_last_exit: NONE,
                    executor_error: NONE,
                    progress_seq: 1,
                    created_at: time::now(),
                    updated_at: time::now()
                }",
            )
            .bind(("key", key))
            .bind(("id", operation_id))
            .bind(("dependencies", dependencies))
            .bind(("payload_hash", payload_hash))
            .bind(("payload", payload))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    let router = api::build_router_with_query_timeout(
        Arc::clone(&storage) as Arc<dyn MemoryStorage>,
        embedder,
        Duration::from_secs(10),
    );

    let receipts = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut committed = Vec::new();
            for operation_id in ["a-dependent", "z-prerequisite"] {
                let response = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(format!("/api/v2/operations/{operation_id}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let body: Value = serde_json::from_slice(
                    &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                )
                .unwrap();
                committed.push(body);
            }
            if committed
                .iter()
                .all(|receipt| receipt["state"] == "committed")
            {
                break committed;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup reconciliation commits both drain waves");

    assert_eq!(receipts[0]["operation_id"], "a-dependent");
    assert_eq!(receipts[1]["operation_id"], "z-prerequisite");
}

fn spawn_persistent_server(address: SocketAddrV4, path: &std::path::Path) -> ServerGuard {
    let child = Command::new("surreal")
        .args([
            "start",
            "--no-banner",
            "--unauthenticated",
            "--allow-all",
            "--bind",
            &address.to_string(),
            &format!("rocksdb:{}", path.display()),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("installed surreal CLI starts the persistent fixture");
    ServerGuard(child)
}

async fn wait_reachable(address: SocketAddrV4) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("persistent surreal fixture becomes reachable");
}

fn task_stream_request(operation_id: &str) -> Value {
    let payload = json!({
        "name": operation_id,
        "description": "ledger restart fixture",
        "agent_id": null,
        "user_id": "test"
    });
    let payload_hash = Sha256::digest(serde_json::to_vec(&payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    json!({
        "operation_id": operation_id,
        "schema_version": 2,
        "kind": "create_task_stream",
        "dependencies": [],
        "payload_hash": payload_hash,
        "payload": payload
    })
}

/// Submit an operation and poll its receipt until it commits. Transient
/// errors while the database restarts are tolerated, but a session that
/// lost its namespace selection (#35) must never surface.
async fn submit_until_committed(router: &axum::Router, operation_id: &str) {
    let request_body = task_stream_request(operation_id);
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut submitted = false;
        loop {
            let response = if submitted {
                router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(format!("/api/v2/operations/{operation_id}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap()
            } else {
                router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/v2/operations")
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                            .unwrap(),
                    )
                    .await
                    .unwrap()
            };
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                !text.contains("Specify a namespace") && !text.contains("Specify a database"),
                "ledger session lost its namespace selection: {text}"
            );
            if status.is_success() {
                submitted = true;
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                if body["state"] == "committed" {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{operation_id} did not commit"));
}

#[tokio::test]
async fn ledger_survives_a_database_restart_without_losing_its_namespace() {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V4(address) = listener.local_addr().unwrap() else {
        unreachable!("bound an IPv4 address")
    };
    drop(listener);
    let data = std::env::temp_dir().join(format!("ledger-restart-{}", uuid::Uuid::new_v4()));
    let server = spawn_persistent_server(address, &data);
    wait_reachable(address).await;

    let embedder: Arc<dyn EmbeddingService> = Arc::new(NoOpEmbedder);
    let storage = Arc::new(
        SurrealStorage::new(
            &SurrealConfig {
                auth_level: Default::default(),
                mode: SurrealMode::Server,
                endpoint: Some(format!("ws://{address}")),
                embedded_path: None,
                username: None,
                password: None,
                namespace: format!("restart_{}", uuid::Uuid::new_v4().simple()),
                database: "operations".to_owned(),
                retry: RetryConfig {
                    max_connect_retries: 0,
                    query_timeout_ms: 10_000,
                    ..RetryConfig::default()
                },
            },
            Arc::clone(&embedder),
        )
        .await
        .expect("server-mode SurrealStorage"),
    );
    let router = api::build_router_with_query_timeout(
        Arc::clone(&storage) as Arc<dyn MemoryStorage>,
        embedder,
        Duration::from_secs(2),
    );
    submit_until_committed(&router, "before-restart").await;

    drop(server);
    let _server = spawn_persistent_server(address, &data);
    wait_reachable(address).await;

    submit_until_committed(&router, "after-restart").await;
}
