//! Verified caller scope at the HTTP boundary (issue #51), exercised through the
//! real router with JWT verification over real per-tenant storage.
//!
//! Isolation model under test: one database per tenant, chosen only from the
//! verified token. Run the same suite against server mode with
//! `TEST_SURREAL_ENDPOINT=ws://127.0.0.1:28100 cargo test --test auth_boundary
//! `.
//! Without `TEST_SURREAL_ENDPOINT` the suite runs on isolated embedded engines.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use futures_util::StreamExt;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use surreal_memory::storage::surreal::{SurrealConfig, SurrealMode};
use surreal_memory::{EmbeddingService, Memory, MemoryStorage, SurrealStorage};
use surreal_memory_server::{
    api::build_router_with_access,
    auth::{Access, JwtVerifier, TenantId, TenantRegistry, TenantStorageFactory},
};
use surrealdb::types::RecordIdKey;
use tower::ServiceExt;

const SECRET: &str = "integration-secret-of-at-least-32-bytes";
const ISSUER: &str = "gate";
const AUDIENCE: &str = "memory";
const DIMENSIONS: usize = 8;

/// Unit vector `degrees` from axis 0 towards `axis`.
fn at(axis: usize, degrees: f32) -> Vec<f32> {
    let radians = degrees.to_radians();
    let mut vector = vec![0.0f32; DIMENSIONS];
    vector[0] = radians.cos();
    vector[axis] = radians.sin();
    vector
}

struct TableEmbedder(HashMap<String, Vec<f32>>);

#[async_trait::async_trait]
impl EmbeddingService for TableEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.0
            .get(text)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no test vector for {text:?}"))
    }
    async fn embed_batch(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut out = Vec::new();
        for text in texts {
            out.push(self.embed(&text).await?);
        }
        Ok(out)
    }
    fn dimensions(&self) -> usize {
        DIMENSIONS
    }
}

/// Every text the tests embed, each on its own axis so nothing merges by accident,
/// except the deliberate near-duplicate pair.
fn embedder() -> Arc<dyn EmbeddingService> {
    let texts = [
        "alpha secret",
        "beta secret",
        "b private note",
        "u1 own note",
        "u2 own note",
        "u1 added by restricted caller",
        "search probe",
        "tenant wide note",
    ];
    let mut table: HashMap<String, Vec<f32>> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| (text.to_string(), at(index % 7 + 1, 60.0 + index as f32)))
        .collect();
    // The near-duplicate pair: cosine 0.985, above the 0.92 merge threshold.
    table.insert("shared fact".into(), at(1, 0.0));
    table.insert("shared fact restated".into(), at(1, 10.0));
    Arc::new(TableEmbedder(table))
}

struct World {
    router: Router,
    /// The storage each tenant's database resolved to, by tenant id.
    tenants: Arc<Mutex<HashMap<String, Arc<SurrealStorage>>>>,
}

impl World {
    fn storage(&self, tenant: &str) -> Arc<SurrealStorage> {
        self.tenants
            .lock()
            .unwrap()
            .get(tenant)
            .cloned()
            .unwrap_or_else(|| panic!("tenant {tenant} was never resolved"))
    }

    fn resolved_tenants(&self) -> usize {
        self.tenants.lock().unwrap().len()
    }
}

async fn world() -> World {
    let embedder = embedder();
    let tenants: Arc<Mutex<HashMap<String, Arc<SurrealStorage>>>> = Arc::default();
    let factory_tenants = Arc::clone(&tenants);
    let factory_embedder = Arc::clone(&embedder);
    // Unique per run so a server-mode rerun never meets an earlier run's rows.
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let run = format!(
        "authtest_{}_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    );
    let factory: TenantStorageFactory = Arc::new(move |tenant: TenantId| {
        let tenants = Arc::clone(&factory_tenants);
        let embedder = Arc::clone(&factory_embedder);
        let run = run.clone();
        Box::pin(async move {
            let storage = Arc::new(match std::env::var("TEST_SURREAL_ENDPOINT") {
                // Server mode: one namespace per tenant, as in production.
                Ok(endpoint) => {
                    let config = SurrealConfig {
                        mode: SurrealMode::Server,
                        endpoint: Some(endpoint),
                        username: Some("root".into()),
                        password: Some("root".into()),
                        namespace: format!("{run}_{tenant}"),
                        database: "main".into(),
                        ..Default::default()
                    };
                    SurrealStorage::new(&config, embedder)
                        .await
                        .map_err(|error| {
                            eprintln!("server tenant storage failed: {error:#}");
                            error
                        })?
                }
                // Embedded mode: an isolated in-memory engine per tenant.
                Err(_) => SurrealStorage::new_mem(embedder).await?,
            });
            tenants
                .lock()
                .unwrap()
                .insert(tenant.as_str().to_string(), Arc::clone(&storage));
            Ok(storage as Arc<dyn MemoryStorage>)
        })
    });
    let registry = Arc::new(TenantRegistry::new(
        factory,
        Arc::clone(&embedder),
        Duration::from_secs(10),
        None,
    ));
    let verifier = Arc::new(JwtVerifier::new(
        Algorithm::HS256,
        DecodingKey::from_secret(SECRET.as_bytes()),
        ISSUER,
        AUDIENCE,
        300,
    ));
    // The base storage must never serve a request in JWT mode.
    let base: Arc<dyn MemoryStorage> = Arc::new(
        SurrealStorage::new_mem(Arc::clone(&embedder))
            .await
            .expect("base storage"),
    );
    let router = build_router_with_access(
        base,
        embedder,
        Duration::from_secs(10),
        Access::Jwt {
            verifier,
            tenants: registry,
        },
    );
    World { router, tenants }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn mint_with(secret: &str, claims: Value) -> String {
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

fn claims(tenant: &str, sub: &str, ops: &[&str], user_id: Option<&str>) -> Value {
    let mut value = json!({
        "iss": ISSUER, "aud": AUDIENCE, "sub": sub, "tenant": tenant,
        "iat": now(), "exp": now() + 600, "ops": ops,
    });
    if let Some(user) = user_id {
        value["user_id"] = json!(user);
    }
    value
}

fn mint(tenant: &str, sub: &str, ops: &[&str], user_id: Option<&str>) -> String {
    mint_with(SECRET, claims(tenant, sub, ops, user_id))
}

/// A tenant-wide service credential.
fn service(tenant: &str) -> String {
    mint(tenant, &format!("svc-{tenant}"), &[], None)
}

fn operator(tenant: &str) -> String {
    mint(tenant, &format!("op-{tenant}"), &["operator"], None)
}

struct Reply {
    status: StatusCode,
    body: Value,
}

async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
    peer: Option<&str>,
    headers: &[(&str, &str)],
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .unwrap();
    if let Some(peer) = peer {
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    }
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn get(router: &Router, uri: &str, token: &str) -> Reply {
    send(router, "GET", uri, Some(token), None, None, &[]).await
}

async fn post(router: &Router, uri: &str, token: &str, body: Value) -> Reply {
    send(router, "POST", uri, Some(token), Some(body), None, &[]).await
}

fn operation(id: &str, kind: &str, dependencies: &[&str], payload: Value) -> Value {
    let hash = Sha256::digest(serde_json::to_vec(&payload).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    json!({
        "operation_id": id, "schema_version": 2, "kind": kind,
        "dependencies": dependencies, "payload_hash": hash, "payload": payload,
    })
}

async fn wait_committed(router: &Router, id: &str, token: &str) -> Value {
    for _ in 0..200 {
        let reply = get(router, &format!("/api/v2/operations/{id}"), token).await;
        if reply.body["state"].as_str() == Some("committed") {
            return reply.body;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("operation {id} never committed");
}

/// First chunk of a streaming response, bounded in time.
async fn first_chunk(router: &Router, request: Request<Body>) -> (StatusCode, String) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let mut stream = response.into_body().into_data_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .ok()
        .flatten()
        .and_then(Result::ok)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    (status, chunk)
}

/// The REST path id (`memory:<key>`) of the stored memory with this content.
async fn id_for(world: &World, tenant: &str, content: &str) -> String {
    let rows = memories_of(world, tenant).await;
    let memory: &Memory = rows
        .iter()
        .find(|memory| memory.content == content)
        .unwrap_or_else(|| panic!("no memory {content:?} in {tenant}"));
    let id = memory.id.as_ref().expect("stored memory has an id");
    let key = match &id.key {
        RecordIdKey::String(value) => value.clone(),
        other => format!("{other:?}"),
    };
    format!("{}:{}", id.table.as_str(), key)
}

async fn memories_of(world: &World, tenant: &str) -> Vec<Memory> {
    world
        .storage(tenant)
        .get_all_memories(None, None, None)
        .await
        .unwrap()
}

// ── Two tenants, shared-looking names, the same client operation ID ──────────

#[tokio::test]
async fn two_tenants_with_the_same_operation_id_commit_independently() {
    let world = world().await;
    let (a, b) = (service("acme"), service("globex"));

    // Same operation ID, same user and agent names, different content.
    for (token, content) in [(&a, "alpha secret"), (&b, "beta secret")] {
        let reply = post(
            &world.router,
            "/api/v2/operations",
            token,
            operation(
                "op-1",
                "add_memory",
                &[],
                json!({"content": content, "user_id": "shared-user", "agent_id": "shared-agent"}),
            ),
        )
        .await;
        assert!(
            matches!(reply.status, StatusCode::ACCEPTED | StatusCode::OK),
            "{reply:?}",
            reply = reply.body
        );
    }

    let receipt_a = wait_committed(&world.router, "op-1", &a).await;
    let receipt_b = wait_committed(&world.router, "op-1", &b).await;
    assert_ne!(
        receipt_a["result"], receipt_b["result"],
        "each tenant must have its own committed result"
    );

    let mem_a = memories_of(&world, "acme").await;
    let mem_b = memories_of(&world, "globex").await;
    assert_eq!(mem_a.len(), 1);
    assert_eq!(mem_a[0].content, "alpha secret");
    assert_eq!(mem_b.len(), 1);
    assert_eq!(mem_b[0].content, "beta secret");

    // Events are tenant-scoped too: A's stream names only A's committed work.
    let request = Request::builder()
        .uri("/api/v2/operations/op-1/events")
        .header("authorization", format!("Bearer {a}"))
        .body(Body::empty())
        .unwrap();
    let (status, chunk) = first_chunk(&world.router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert!(chunk.contains("op-1"), "{chunk}");
    assert!(!chunk.contains("beta secret"), "{chunk}");
}

// ── Route inventory: nothing is reachable without a verified credential ─────

const INVENTORY: &[(&str, &str)] = &[
    ("GET", "/api/v2/operations/stats"),
    ("POST", "/api/v2/operations"),
    ("GET", "/api/v2/operations/op-x"),
    ("GET", "/api/v2/operations/op-x/events"),
    ("POST", "/api/v2/operations/op-x/retry"),
    ("POST", "/api/v2/operations/op-x/reject"),
    ("POST", "/api/v1/memory"),
    ("GET", "/api/v1/memory"),
    ("DELETE", "/api/v1/memory"),
    ("GET", "/api/v1/memory/m1"),
    ("PUT", "/api/v1/memory/m1"),
    ("DELETE", "/api/v1/memory/m1"),
    ("GET", "/api/v1/memory/m1/history"),
    ("POST", "/api/v1/search"),
    ("GET", "/api/v1/entities"),
    ("POST", "/api/v1/entities/batch"),
    ("GET", "/api/v1/entities/e1"),
    ("POST", "/api/v1/entities/e1/observations"),
    ("GET", "/api/v1/entities/e1/neighbors"),
    ("GET", "/api/v1/entities/e1/related"),
    ("GET", "/api/v1/entities/e1/path/e2"),
    ("POST", "/api/v1/entities/relations"),
    ("POST", "/api/v1/entities/relations/batch"),
    ("GET", "/api/v1/taskstreams"),
    ("GET", "/api/v1/mindmaps"),
    ("GET", "/api/v1/mindmaps/m1"),
    ("POST", "/api/v1/mindmaps/generate/persona"),
    ("POST", "/api/v1/mindmaps/generate/ideation"),
    ("POST", "/api/v1/mindmaps/m1/nodes"),
    ("DELETE", "/api/v1/mindmaps/m1/nodes/n1"),
    ("POST", "/api/v1/mindmaps/m1/edges"),
    ("GET", "/api/v1/mindmaps/m1/export"),
    ("GET", "/api/v1/palace/status"),
    ("POST", "/api/v1/palace/wake"),
    ("POST", "/api/v1/palace/recall"),
    ("POST", "/api/v1/palace/ingest"),
    ("DELETE", "/api/v1/palace/drawer/d1"),
    ("POST", "/api/v1/palace/hybrid-search"),
    ("POST", "/api/v1/taskstreams/t1/archive"),
    ("POST", "/api/v1/taskstreams/t1/pause"),
    ("POST", "/api/v1/taskstreams/t1/memories"),
    ("GET", "/api/v1/taskstreams/t1/context"),
    ("POST", "/api/v1/taskstreams/t1/summarize"),
    ("GET", "/agent.json"),
    ("POST", "/a2a/tasks/send"),
    ("GET", "/a2a/tasks/t1"),
    ("POST", "/mcp/http"),
    ("GET", "/mcp/sse"),
    ("POST", "/mcp/messages?sessionId=s1"),
    ("GET", "/api/v1/unlisted-future-route"),
];

#[tokio::test]
async fn every_data_route_fails_before_data_access_without_a_valid_credential() {
    let world = world().await;
    let stale = {
        let mut stale = claims("acme", "svc", &[], None);
        stale["iat"] = json!(now() - 3_600);
        mint_with(SECRET, stale)
    };
    let expired = {
        let mut expired = claims("acme", "svc", &[], None);
        expired["iat"] = json!(now() - 3_600);
        expired["exp"] = json!(now() - 1_800);
        mint_with(SECRET, expired)
    };
    let forged = mint_with(
        "another-secret-of-at-least-32-bytes!",
        claims("acme", "svc", &[], None),
    );
    let wrong_audience = {
        let mut wrong = claims("acme", "svc", &[], None);
        wrong["aud"] = json!("another-service");
        mint_with(SECRET, wrong)
    };
    let credentials: [(&str, Option<&str>); 6] = [
        ("missing", None),
        ("garbage", Some("not-a-token")),
        ("expired", Some(&expired)),
        ("stale (revoked grant not re-issued)", Some(&stale)),
        ("forged signature", Some(&forged)),
        ("wrong audience", Some(&wrong_audience)),
    ];

    for (method, uri) in INVENTORY {
        for (label, token) in credentials {
            let reply = send(
                &world.router,
                method,
                uri,
                token,
                Some(json!({})),
                None,
                &[],
            )
            .await;
            assert_eq!(
                reply.status,
                StatusCode::UNAUTHORIZED,
                "{method} {uri} with {label} credential"
            );
        }
    }
    assert_eq!(
        world.resolved_tenants(),
        0,
        "no tenant storage may be touched before authentication succeeds"
    );

    for uri in ["/health", "/ready"] {
        let reply = send(&world.router, "GET", uri, None, None, None, &[]).await;
        assert_ne!(reply.status, StatusCode::UNAUTHORIZED, "{uri} is public");
    }
}

/// Every route declared in the sources must be classified in `INVENTORY`, so a
/// route added later fails here until someone classifies it.
#[test]
fn every_declared_route_is_in_the_inventory() {
    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    rust_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );

    let public = ["/health", "/ready"];
    let mut unclassified = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        // Ignore unit-test modules, which mount routers under test paths.
        let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
        for marker in [".route(\"", ".nest(\"", ".nest_service(\""] {
            for part in production.split(marker).skip(1) {
                let path = part.split('"').next().unwrap_or_default();
                if path == "/" || public.contains(&path) {
                    continue;
                }
                // Compare the static path segments, in order; `{param}` segments
                // match any value.
                let wanted: Vec<&str> = path
                    .split('/')
                    .filter(|segment| !segment.is_empty() && !segment.starts_with('{'))
                    .collect();
                if wanted.is_empty() {
                    continue;
                }
                let known = INVENTORY.iter().any(|(_, uri)| {
                    let mut have = uri.split('?').next().unwrap_or_default().split('/');
                    wanted
                        .iter()
                        .all(|segment| have.any(|part| part == *segment))
                });
                if !known {
                    unclassified.push(format!("{}: {path}", file.display()));
                }
            }
        }
    }
    assert!(
        unclassified.is_empty(),
        "classify these routes in INVENTORY: {unclassified:#?}"
    );
}

// ── Tenant A cannot reach tenant B ───────────────────────────────────────────

#[tokio::test]
async fn one_tenant_cannot_read_change_or_enumerate_another() {
    let world = world().await;
    let (a, b) = (service("acme"), service("globex"));

    let created = post(
        &world.router,
        "/api/v1/memory",
        &b,
        json!({"content": "b private note", "user_id": "u1", "agent_id": "agent-1"}),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let b_id = id_for(&world, "globex", "b private note").await;
    let b_uri = format!("/api/v1/memory/{b_id}");

    let before = memories_of(&world, "globex").await;

    // A guesses B's ID and asks for B's scope explicitly, by omission, and empty.
    assert_eq!(
        get(&world.router, &b_uri, &a).await.status,
        StatusCode::NOT_FOUND
    );
    let history = get(&world.router, &format!("{b_uri}/history"), &a).await;
    assert_eq!(
        history.body,
        json!([]),
        "none of B's history is visible to A"
    );
    let put = send(
        &world.router,
        "PUT",
        &b_uri,
        Some(&a),
        Some(json!({"content": "hijacked"})),
        None,
        &[],
    )
    .await;
    assert_eq!(put.status, StatusCode::NOT_FOUND, "{:?}", put.body);
    let delete = send(&world.router, "DELETE", &b_uri, Some(&a), None, None, &[]).await;
    assert_eq!(delete.status, StatusCode::NOT_FOUND);
    for scope in ["?user_id=u1&agent_id=agent-1", "", "?user_id="] {
        let list = get(&world.router, &format!("/api/v1/memory{scope}"), &a).await;
        assert_eq!(
            list.body,
            json!([]),
            "scope {scope:?} must show nothing of B"
        );
    }
    let wipe = send(
        &world.router,
        "DELETE",
        "/api/v1/memory?user_id=u1",
        Some(&a),
        None,
        None,
        &[],
    )
    .await;
    assert_eq!(wipe.body["deleted"], json!(0));

    // B's rows are unchanged.
    let after = memories_of(&world, "globex").await;
    assert_eq!(before.len(), after.len());
    assert_eq!(after[0].content, "b private note");
    assert_eq!(after[0].version, before[0].version);
}

#[tokio::test]
async fn operations_receipts_stats_and_dependencies_do_not_cross_tenants() {
    let world = world().await;
    let (a, b) = (service("acme"), service("globex"));
    let (a_op, b_op) = (operator("acme"), operator("globex"));

    let queued = post(
        &world.router,
        "/api/v2/operations",
        &b,
        operation(
            "only-b",
            "add_memory",
            &[],
            json!({"content": "beta secret", "user_id": "u1"}),
        ),
    )
    .await;
    assert!(queued.status.is_success());
    wait_committed(&world.router, "only-b", &b).await;

    assert_eq!(
        get(&world.router, "/api/v2/operations/only-b", &a)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&world.router, "/api/v2/operations/only-b/events", &a)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            &world.router,
            "/api/v2/operations/only-b/retry",
            &a_op,
            json!({})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            &world.router,
            "/api/v2/operations/only-b/reject",
            &a_op,
            json!({"reason": "x"})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    // B's operation is unchanged by A's operator attempts.
    let receipt = get(&world.router, "/api/v2/operations/only-b", &b_op).await;
    assert_eq!(receipt.body["state"], json!("committed"));

    // A's stats count nothing of B's work.
    let stats_a = get(&world.router, "/api/v2/operations/stats", &a).await;
    let stats_b = get(&world.router, "/api/v2/operations/stats", &b).await;
    assert_ne!(stats_a.body, stats_b.body);

    // A dependency on B's operation ID is not satisfied by B's committed work.
    let dependent = post(
        &world.router,
        "/api/v2/operations",
        &a,
        operation(
            "a-depends",
            "add_memory",
            &["only-b"],
            json!({"content": "alpha secret", "user_id": "u1"}),
        ),
    )
    .await;
    assert!(dependent.status.is_success() || dependent.status == StatusCode::BAD_REQUEST);
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let state = get(&world.router, "/api/v2/operations/a-depends", &a).await;
    assert_ne!(state.body["state"], json!("committed"), "{:?}", state.body);
    assert!(memories_of(&world, "acme").await.is_empty());
}

// ── MCP sessions are bound to their principal ────────────────────────────────

#[tokio::test]
async fn legacy_sse_sessions_cannot_be_used_by_another_principal() {
    let world = world().await;
    let owner = mint("acme", "svc-owner", &[], None);
    let other_tenant = service("globex");
    let same_tenant_other_subject = mint("acme", "svc-other", &[], None);

    let request = Request::builder()
        .uri("/mcp/sse")
        .header("authorization", format!("Bearer {owner}"))
        .header("host", "localhost:3001")
        .body(Body::empty())
        .unwrap();
    let (status, chunk) = first_chunk(&world.router, request).await;
    assert_eq!(status, StatusCode::OK);
    let session = chunk
        .split("sessionId=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("endpoint event carries a session id")
        .to_string();

    let message = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let uri = format!("/mcp/messages?sessionId={session}");
    for (label, token) in [
        ("other tenant", &other_tenant),
        ("same tenant, other subject", &same_tenant_other_subject),
    ] {
        let reply = post(&world.router, &uri, token, message.clone()).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{label} must not reuse the session"
        );
    }
    let own = post(&world.router, &uri, &owner, message).await;
    assert_eq!(own.status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn streamable_sessions_cannot_be_used_by_another_principal() {
    let world = world().await;
    let owner = mint("acme", "svc-owner", &[], None);
    let other = service("globex");

    let initialize = Request::builder()
        .method("POST")
        .uri("/mcp/http")
        .header("authorization", format!("Bearer {owner}"))
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}}})
            .to_string(),
        ))
        .unwrap();
    let response = world.router.clone().oneshot(initialize).await.unwrap();
    let status = response.status();
    if status != StatusCode::OK {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        panic!("initialize: {status} {}", String::from_utf8_lossy(&body));
    }
    let session = response
        .headers()
        .get("mcp-session-id")
        .expect("session id header")
        .to_str()
        .unwrap()
        .to_string();

    let follow_up = |token: &str| {
        Request::builder()
            .method("POST")
            .uri("/mcp/http")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("host", "localhost")
            .header("mcp-session-id", session.clone())
            .body(Body::from(
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            ))
            .unwrap()
    };
    let stolen = world
        .router
        .clone()
        .oneshot(follow_up(&other))
        .await
        .unwrap();
    assert_eq!(
        stolen.status(),
        StatusCode::NOT_FOUND,
        "another principal borrowed the session"
    );
    let own = world
        .router
        .clone()
        .oneshot(follow_up(&owner))
        .await
        .unwrap();
    assert_eq!(own.status(), StatusCode::ACCEPTED);
}

// ── Forged proxy headers are not credentials ─────────────────────────────────

#[tokio::test]
async fn forged_identity_and_proxy_headers_grant_nothing() {
    let world = world().await;
    let forged = [
        ("x-user-id", "u1"),
        ("x-tenant-id", "globex"),
        ("x-forwarded-for", "127.0.0.1"),
        ("x-real-ip", "127.0.0.1"),
        ("forwarded", "for=127.0.0.1"),
    ];
    let reply = send(
        &world.router,
        "GET",
        "/api/v1/memory",
        None,
        None,
        Some("127.0.0.1:1234"),
        &forged,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    // With a real token for acme, a forged tenant header does not redirect it.
    let a = service("acme");
    let reply = send(
        &world.router,
        "GET",
        "/api/v1/memory",
        Some(&a),
        None,
        None,
        &forged,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(world.resolved_tenants(), 1);
    world.storage("acme");
}

// ── User-restricted credentials ──────────────────────────────────────────────

async fn seed_two_users(world: &World) -> (String, String) {
    let tenant_wide = service("acme");
    for (content, user) in [("u1 own note", "u1"), ("u2 own note", "u2")] {
        let created = post(
            &world.router,
            "/api/v1/memory",
            &tenant_wide,
            json!({"content": content, "user_id": user, "agent_id": "agent-x"}),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED);
    }
    (
        id_for(world, "acme", "u1 own note").await,
        id_for(world, "acme", "u2 own note").await,
    )
}

#[tokio::test]
async fn a_user_restricted_credential_sees_and_changes_only_its_own_memories() {
    let world = world().await;
    let (u1_id, u2_id) = seed_two_users(&world).await;
    let u1 = mint("acme", "user-1", &[], Some("u1"));
    let u2_uri = format!("/api/v1/memory/{u2_id}");

    // Omitted scope defaults to its own user; naming another (or empty) is refused.
    let own = get(&world.router, "/api/v1/memory", &u1).await;
    assert_eq!(own.body.as_array().unwrap().len(), 1);
    assert_eq!(own.body[0]["content"], json!("u1 own note"));
    for scope in ["?user_id=u2", "?user_id="] {
        let list = get(&world.router, &format!("/api/v1/memory{scope}"), &u1).await;
        assert_eq!(list.status, StatusCode::FORBIDDEN, "scope {scope:?}");
    }
    let search = post(
        &world.router,
        "/api/v1/search",
        &u1,
        json!({"query": "search probe", "user_id": "u2"}),
    )
    .await;
    assert_eq!(search.status, StatusCode::FORBIDDEN);
    let write = post(
        &world.router,
        "/api/v1/memory",
        &u1,
        json!({"content": "u1 added by restricted caller", "user_id": "u2"}),
    )
    .await;
    assert_eq!(write.status, StatusCode::FORBIDDEN);
    let wipe = send(
        &world.router,
        "DELETE",
        "/api/v1/memory?user_id=u2",
        Some(&u1),
        None,
        None,
        &[],
    )
    .await;
    assert_eq!(wipe.status, StatusCode::FORBIDDEN);

    // By-ID access to another user's memory looks like a missing one.
    assert_eq!(
        get(&world.router, &u2_uri, &u1).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&world.router, &format!("{u2_uri}/history"), &u1)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let put = send(
        &world.router,
        "PUT",
        &u2_uri,
        Some(&u1),
        Some(json!({"content": "hijacked"})),
        None,
        &[],
    )
    .await;
    assert_eq!(put.status, StatusCode::NOT_FOUND);
    let delete = send(&world.router, "DELETE", &u2_uri, Some(&u1), None, None, &[]).await;
    assert_eq!(delete.status, StatusCode::NOT_FOUND);

    // u2's memory and its history are untouched.
    let rows = memories_of(&world, "acme").await;
    let u2 = rows
        .iter()
        .find(|m| m.user_id.as_deref() == Some("u2"))
        .expect("u2 row survives");
    assert_eq!(u2.content, "u2 own note");
    assert_eq!(u2.version, 1);

    // The restricted caller can still manage its own memory.
    let renamed = send(
        &world.router,
        "PUT",
        &format!("/api/v1/memory/{u1_id}"),
        Some(&u1),
        Some(json!({"content": "u1 added by restricted caller"})),
        None,
        &[],
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
}

#[tokio::test]
async fn a_user_restricted_credential_is_limited_to_the_memory_and_search_routes() {
    let world = world().await;
    let u1 = mint("acme", "user-1", &[], Some("u1"));
    for (method, uri) in [
        ("GET", "/api/v1/entities"),
        ("GET", "/api/v1/taskstreams"),
        ("GET", "/api/v1/mindmaps"),
        ("POST", "/api/v2/operations"),
        ("GET", "/api/v2/operations/stats"),
        ("POST", "/mcp/http"),
        ("GET", "/mcp/sse"),
        ("POST", "/a2a/tasks/send"),
        ("GET", "/agent.json"),
        ("GET", "/api/v1/unlisted-future-route"),
    ] {
        let reply = send(
            &world.router,
            method,
            uri,
            Some(&u1),
            Some(json!({})),
            None,
            &[],
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[tokio::test]
async fn a_missing_agent_id_cannot_merge_into_another_users_attributed_memory() {
    let world = world().await;
    let tenant_wide = service("acme");
    // u2's attributed memory.
    let seeded = post(
        &world.router,
        "/api/v1/memory",
        &tenant_wide,
        json!({"content": "shared fact", "user_id": "u2", "agent_id": "agent-x"}),
    )
    .await;
    assert_eq!(seeded.status, StatusCode::CREATED);

    // u1 restates it without an agent_id; cosine similarity is 0.985 (> 0.92).
    let u1 = mint("acme", "user-1", &[], Some("u1"));
    let restated = post(
        &world.router,
        "/api/v1/memory",
        &u1,
        json!({"content": "shared fact restated"}),
    )
    .await;
    assert_eq!(restated.status, StatusCode::CREATED);

    let rows = memories_of(&world, "acme").await;
    assert_eq!(rows.len(), 2, "u1's write must create its own memory");
    let u2 = rows
        .iter()
        .find(|m| m.user_id.as_deref() == Some("u2"))
        .unwrap();
    assert_eq!(u2.content, "shared fact");
    assert_eq!(u2.agent_id.as_deref(), Some("agent-x"));
    assert_eq!(u2.version, 1);
    let u1_row = rows
        .iter()
        .find(|m| m.user_id.as_deref() == Some("u1"))
        .unwrap();
    assert_eq!(u1_row.content, "shared fact restated");
}

// ── Operator capability ──────────────────────────────────────────────────────

#[tokio::test]
async fn operator_endpoints_need_the_operator_permission_within_the_granted_tenant() {
    let world = world().await;
    let ordinary = service("acme");
    let operator_token = operator("acme");
    let other_operator = operator("globex");

    let submitted = post(
        &world.router,
        "/api/v2/operations",
        &ordinary,
        operation(
            "acme-op",
            "add_memory",
            &[],
            json!({"content": "alpha secret", "user_id": "u1"}),
        ),
    )
    .await;
    assert!(submitted.status.is_success());
    wait_committed(&world.router, "acme-op", &ordinary).await;

    assert_eq!(
        post(
            &world.router,
            "/api/v2/operations/acme-op/retry",
            &ordinary,
            json!({})
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &world.router,
            "/api/v2/operations/acme-op/reject",
            &ordinary,
            json!({"reason": "x"})
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    // An operator of another tenant has no authority over acme's ledger.
    assert_eq!(
        post(
            &world.router,
            "/api/v2/operations/acme-op/retry",
            &other_operator,
            json!({})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    // An acme operator is past the permission check (the committed operation is a conflict, not a 403).
    let retry = post(
        &world.router,
        "/api/v2/operations/acme-op/retry",
        &operator_token,
        json!({}),
    )
    .await;
    assert_ne!(retry.status, StatusCode::FORBIDDEN);

    // Re-key needs the operator permission AND a loopback peer.
    let rekey = operation(
        "rekey-1",
        "rekey_agent_id",
        &[],
        json!({"from": "agent-x", "to": "agent-y", "dry_run": true}),
    );
    let denied_ordinary = send(
        &world.router,
        "POST",
        "/api/v2/operations",
        Some(&ordinary),
        Some(rekey.clone()),
        Some("127.0.0.1:1"),
        &[],
    )
    .await;
    assert_eq!(denied_ordinary.status, StatusCode::FORBIDDEN);
    let denied_peer = send(
        &world.router,
        "POST",
        "/api/v2/operations",
        Some(&operator_token),
        Some(rekey.clone()),
        Some("10.0.0.5:1"),
        &[],
    )
    .await;
    assert_eq!(denied_peer.status, StatusCode::FORBIDDEN);
    let denied_unknown_peer = send(
        &world.router,
        "POST",
        "/api/v2/operations",
        Some(&operator_token),
        Some(rekey.clone()),
        None,
        &[],
    )
    .await;
    assert_eq!(denied_unknown_peer.status, StatusCode::FORBIDDEN);
    let allowed = send(
        &world.router,
        "POST",
        "/api/v2/operations",
        Some(&operator_token),
        Some(rekey),
        Some("127.0.0.1:1"),
        &[],
    )
    .await;
    assert_ne!(allowed.status, StatusCode::FORBIDDEN, "{:?}", allowed.body);
}

// ── Local trusted mode stays explicit and working ────────────────────────────

#[tokio::test]
async fn local_mode_serves_the_trusted_client_without_credentials() {
    let embedder = embedder();
    let storage: Arc<dyn MemoryStorage> = Arc::new(
        SurrealStorage::new_mem(Arc::clone(&embedder))
            .await
            .unwrap(),
    );
    let router = surreal_memory_server::api::build_router(storage, embedder);
    let reply = send(&router, "GET", "/api/v1/memory", None, None, None, &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
}
