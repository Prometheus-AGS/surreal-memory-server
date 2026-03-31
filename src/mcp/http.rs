use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    response::{Sse, sse::Event},
    routing::{get, post},
};
use dashmap::DashMap;
use rmcp::{
    model::ClientJsonRpcMessage,
    serve_server,
    service::{RoleServer, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        Transport, TransportAdapterIdentity,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use uuid::Uuid;

use crate::{mcp::MemoryMcpServer, storage::MemoryStorage};

type SessionMap = Arc<DashMap<String, mpsc::Sender<RxJsonRpcMessage<RoleServer>>>>;

#[derive(Clone)]
struct HttpAppState {
    storage: Arc<dyn MemoryStorage>,
    sessions: SessionMap,
}

pub fn mcp_http_router(storage: Arc<dyn MemoryStorage>, prefix: &str) -> Router {
    let state = HttpAppState {
        storage: storage.clone(),
        sessions: Arc::new(DashMap::new()),
    };

    // --- Streamable HTTP Endpoint (New Standard) ---
    let storage_clone = storage.clone();
    let service = StreamableHttpService::new(
        move || Ok(MemoryMcpServer::new(storage_clone.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let streamable_routes = Router::new().nest_service("/http", service);

    // --- Legacy SSE Endpoints (Backward Compatibility) ---
    let legacy_routes = Router::new()
        .route("/sse", get(sse_handler))
        .route("/messages", post(messages_handler))
        .with_state(state);

    // Combine both routers
    let routes = streamable_routes.merge(legacy_routes);

    Router::new().nest(prefix, routes)
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
    State(state): State<HttpAppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let session_id = Uuid::new_v4().to_string();

    let (client_tx, server_rx) = mpsc::channel::<RxJsonRpcMessage<RoleServer>>(100);
    let (server_tx, client_rx) = mpsc::channel::<TxJsonRpcMessage<RoleServer>>(100);

    state.sessions.insert(session_id.clone(), client_tx);

    let transport = MuxTransport {
        tx: server_tx,
        rx: server_rx,
    };

    let server = MemoryMcpServer::new(state.storage.clone());

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

    Sse::new(combined_stream).keep_alive(axum::response::sse::KeepAlive::new())
}

#[derive(serde::Deserialize)]
struct MessagesQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}

async fn messages_handler(
    State(state): State<HttpAppState>,
    Query(query): Query<MessagesQuery>,
    Json(payload): Json<ClientJsonRpcMessage>,
) -> axum::response::Result<impl axum::response::IntoResponse, axum::http::StatusCode> {
    if let Some(tx) = state.sessions.get(&query.session_id)
        && tx.send(payload).await.is_ok()
    {
        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    Err(axum::http::StatusCode::NOT_FOUND)
}
