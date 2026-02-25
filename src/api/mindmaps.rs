//! Mindmap REST API routes.
//! GET/POST/DELETE /api/v1/mindmaps

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::Deserialize;
use surreal_memory::{MapType, MindMap, MindMapEdge, MindMapNode};

use super::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_mindmap))
        .route("/", get(list_mindmaps))
        .route("/{name}", get(get_mindmap))
        .route("/{name}", delete(delete_mindmap))
        .route("/{name}/nodes", post(add_node))
        .route("/{name}/nodes/{node_id}", delete(delete_node))
        .route("/{name}/edges", post(add_edge))
        .route("/{name}/export", get(export_mindmap))
}

#[derive(Deserialize)]
struct ScopeQuery {
    user_id: Option<String>,
    agent_id: Option<String>,
}

#[derive(Deserialize)]
struct ExportQuery {
    user_id: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}
fn default_format() -> String {
    "json".to_string()
}

#[derive(Deserialize)]
struct CreateMindmapBody {
    name: String,
    map_type: Option<String>,
    root_label: String,
    description: Option<String>,
    agent_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Deserialize)]
struct AddNodeBody {
    node_id: String,
    label: String,
    parent_id: Option<String>,
    node_type: Option<String>,
    color: Option<String>,
}

#[derive(Deserialize)]
struct AddEdgeBody {
    from_id: String,
    to_id: String,
    label: Option<String>,
    #[serde(default)]
    directed: bool,
}

fn internal_err(e: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

async fn create_mindmap(
    State(state): State<AppState>,
    Json(body): Json<CreateMindmapBody>,
) -> Result<(StatusCode, Json<MindMap>), (StatusCode, Json<serde_json::Value>)> {
    let map_type = match body.map_type.as_deref().unwrap_or("radial") {
        "concept" => MapType::Concept,
        "argument" => MapType::Argument,
        "tree" => MapType::Tree,
        "temporal" => MapType::Temporal,
        _ => MapType::Radial,
    };
    let mm = MindMap::new(
        body.name,
        map_type,
        body.root_label,
        body.description,
        body.agent_id,
        body.user_id,
    );
    let created = state
        .storage
        .create_mindmap(mm)
        .await
        .map_err(internal_err)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn list_mindmaps(
    State(state): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Vec<MindMap>>, (StatusCode, Json<serde_json::Value>)> {
    let maps = state
        .storage
        .list_mindmaps(q.user_id.as_deref(), q.agent_id.as_deref())
        .await
        .map_err(internal_err)?;
    Ok(Json(maps))
}

async fn get_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<Option<MindMap>>, (StatusCode, Json<serde_json::Value>)> {
    let mm = state
        .storage
        .get_mindmap(&name, q.user_id.as_deref())
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn delete_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    state
        .storage
        .delete_mindmap(&name, q.user_id.as_deref())
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_node(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
    Json(body): Json<AddNodeBody>,
) -> Result<Json<MindMap>, (StatusCode, Json<serde_json::Value>)> {
    let node = MindMapNode {
        id: body.node_id,
        label: body.label,
        parent_id: body.parent_id,
        node_type: body.node_type,
        color: body.color,
        metadata: None,
    };
    let mm = state
        .storage
        .add_mindmap_node(&name, q.user_id.as_deref(), node)
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn delete_node(
    State(state): State<AppState>,
    Path((name, node_id)): Path<(String, String)>,
    Query(q): Query<ScopeQuery>,
) -> Result<Json<MindMap>, (StatusCode, Json<serde_json::Value>)> {
    let mm = state
        .storage
        .delete_mindmap_node(&name, q.user_id.as_deref(), &node_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn add_edge(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ScopeQuery>,
    Json(body): Json<AddEdgeBody>,
) -> Result<Json<MindMap>, (StatusCode, Json<serde_json::Value>)> {
    let edge = MindMapEdge {
        from_id: body.from_id,
        to_id: body.to_id,
        label: body.label,
        directed: body.directed,
    };
    let mm = state
        .storage
        .add_mindmap_edge(&name, q.user_id.as_deref(), edge)
        .await
        .map_err(internal_err)?;
    Ok(Json(mm))
}

async fn export_mindmap(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    use surreal_memory::ExportFormat;
    let fmt = match q.format.to_lowercase().as_str() {
        "mermaid" => ExportFormat::Mermaid,
        "markdown" | "md" => ExportFormat::Markdown,
        _ => ExportFormat::Json,
    };
    let mm = state
        .storage
        .get_mindmap(&name, q.user_id.as_deref())
        .await
        .map_err(internal_err)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "not found" })),
            )
        })?;
    Ok(mm.export(&fmt))
}
