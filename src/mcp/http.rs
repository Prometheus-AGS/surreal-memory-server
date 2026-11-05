use axum::{
    Json, Router,
    body::Body,
    extract::{Query, Request, State},
    http::HeaderMap,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{any, get, post},
};
use dashmap::DashMap;
use rmcp::{
    model::ClientJsonRpcMessage,
    serve_server,
    service::{RoleServer, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        Transport, TransportAdapterIdentity,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
            session::local::{LocalSessionManager, SessionConfig},
        },
    },
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    api::{ApiFailure, AppState},
    auth::{Caller, Principal, TenantId},
    mcp::MemoryMcpServer,
};

/// Who owns an MCP session: the verified `(tenant, subject)` that opened it.
type SessionOwner = (TenantId, String);

struct LegacySession {
    owner: SessionOwner,
    sender: mpsc::Sender<RxJsonRpcMessage<RoleServer>>,
}

type SessionMap = Arc<DashMap<String, LegacySession>>;

type StreamableService = StreamableHttpService<MemoryMcpServer, LocalSessionManager>;

#[derive(Clone)]
struct McpState {
    root: AppState,
    /// One streamable-HTTP service (and so one session manager) per verified
    /// principal. A session ID presented by anyone else reaches a different
    /// manager and is unknown there, so sessions cannot be borrowed.
    streamable: Arc<DashMap<SessionOwner, StreamableService>>,
    sessions: SessionMap,
}

pub fn mcp_http_router(root: AppState, prefix: &str) -> Router {
    let state = McpState {
        root,
        streamable: Arc::new(DashMap::new()),
        sessions: Arc::new(DashMap::new()),
    };

    let routes = Router::new()
        // Streamable HTTP (current standard).
        .route("/http", any(streamable_handler))
        .route("/http/{*rest}", any(streamable_handler))
        // Legacy SSE (backward compatibility), now owner-bound.
        .route("/sse", get(sse_handler))
        .route("/messages", post(messages_handler))
        .with_state(state);

    Router::new().nest(prefix, routes)
}

fn streamable_service(storage: Arc<dyn crate::storage::MemoryStorage>) -> StreamableService {
    // rmcp 1.4.0's `SessionConfig::DEFAULT_KEEP_ALIVE` is 5 minutes. Idle MCP
    // clients (the common case: a coding agent that calls a tool, thinks for a
    // while, then calls another tool) get culled mid-conversation and the next
    // request returns 404 with `mcp-session-id` set. That manifests upstream as
    // sporadic "list_task_streams timed out" / "add_memory timed out" because
    // the client retries on a dead session. Bump the idle timeout to 24h —
    // long enough that no realistic conversation hits it, short enough that
    // truly abandoned sessions still get reclaimed.
    let mut session_config = SessionConfig::default();
    session_config.keep_alive = Some(Duration::from_secs(60 * 60 * 24));
    let mut session_manager = LocalSessionManager::default();
    session_manager.session_config = session_config;
    StreamableHttpService::new(
        move || Ok(MemoryMcpServer::new(Arc::clone(&storage))),
        Arc::new(session_manager),
        StreamableHttpServerConfig::default(),
    )
}

async fn streamable_handler(
    State(state): State<McpState>,
    Caller(principal): Caller,
    request: Request,
) -> Result<Response, ApiFailure> {
    let owner = principal.session_owner();
    let service = match state.streamable.get(&owner) {
        Some(existing) => existing.clone(),
        None => {
            let scoped = state.root.for_principal(&principal).await?;
            state
                .streamable
                .entry(owner)
                .or_insert_with(|| streamable_service(scoped.storage))
                .clone()
        }
    };
    let response = service
        .oneshot(request)
        .await
        .unwrap_or_else(|never| match never {});
    let (parts, body) = response.into_parts();
    Ok(Response::from_parts(parts, Body::new(body)).into_response())
}

pub struct MuxTransport {
    pub tx: mpsc::Sender<TxJsonRpcMessage<RoleServer>>,
    pub rx: mpsc::Receiver<RxJsonRpcMessage<RoleServer>>,
}

impl Transport<RoleServer> for MuxTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let tx = self.tx.clone();
        async move {
            tx.send(item)
                .await
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "send error"))
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        self.rx.recv().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

async fn sse_handler(
    headers: HeaderMap,
    State(state): State<McpState>,
    Caller(principal): Caller,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiFailure> {
    let scoped = state.root.for_principal(&principal).await?;
    let session_id = Uuid::new_v4().to_string();

    let (client_tx, server_rx) = mpsc::channel::<RxJsonRpcMessage<RoleServer>>(100);
    let (server_tx, client_rx) = mpsc::channel::<TxJsonRpcMessage<RoleServer>>(100);

    state.sessions.insert(
        session_id.clone(),
        LegacySession {
            owner: principal.session_owner(),
            sender: client_tx,
        },
    );

    let transport = MuxTransport {
        tx: server_tx,
        rx: server_rx,
    };

    let server = MemoryMcpServer::new(scoped.storage);

    let session_id_clone = session_id.clone();
    let sessions_clone = state.sessions.clone();
    tokio::spawn(async move {
        let res = serve_server::<MemoryMcpServer, MuxTransport, _, TransportAdapterIdentity>(
            server, transport,
        )
        .await;

        match res {
            Ok(service) => {
                let _ = service.waiting().await;
            }
            Err(e) => {
                tracing::error!("Serve server error: {}", e);
            }
        }

        sessions_clone.remove(&session_id_clone);
        tracing::info!("MCP session {} closed", session_id_clone);
    });

    let stream = ReceiverStream::new(client_rx).map(|msg| {
        let json_str = serde_json::to_string(&msg).unwrap_or_default();
        Ok(Event::default().event("message").data(json_str))
    });

    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3000");

    // Revert host binding behavior correctly for dynamic hosts
    let endpoint_uri = format!("http://{}/mcp/messages?sessionId={}", host, session_id);
    let initial_stream =
        tokio_stream::once(Ok(Event::default().event("endpoint").data(endpoint_uri)));

    let combined_stream = initial_stream.chain(stream);

    Ok(Sse::new(combined_stream).keep_alive(axum::response::sse::KeepAlive::new()))
}

#[derive(serde::Deserialize)]
struct MessagesQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}

async fn messages_handler(
    State(state): State<McpState>,
    Caller(principal): Caller,
    Query(query): Query<MessagesQuery>,
    Json(payload): Json<ClientJsonRpcMessage>,
) -> axum::response::Result<impl axum::response::IntoResponse, axum::http::StatusCode> {
    // A session owned by someone else is reported exactly like an unknown one.
    let sender = owned_session_sender(&state.sessions, &query.session_id, &principal);
    if let Some(tx) = sender
        && tx.send(payload).await.is_ok()
    {
        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    Err(axum::http::StatusCode::NOT_FOUND)
}

fn owned_session_sender(
    sessions: &SessionMap,
    session_id: &str,
    principal: &Principal,
) -> Option<mpsc::Sender<RxJsonRpcMessage<RoleServer>>> {
    let session = sessions.get(session_id)?;
    (session.owner == principal.session_owner()).then(|| session.sender.clone())
}
