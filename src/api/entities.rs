//! Entity REST API routes.
//! GET/POST/DELETE /api/v1/entities

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::Deserialize;
use surreal_memory::{Entity, Relation};

use super::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_entity))
        .route("/", get(get_graph))
        .route("/batch", post(create_entities_batch))
        .route("/{name}", delete(delete_entity))
        .route("/{name}/observations", post(add_observations))
        .route("/{name}/neighbors", get(expand_neighbors))
        .route("/{name}/related", get(get_related))
        .route("/{name}/path/{to}", get(find_path))
        .route("/relations", post(create_relation))
        .route("/relations/batch", post(create_relations_batch))
        .route("/search", get(search_entities))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

#[derive(Deserialize)]
struct CreateEntityBody {
    name: String,
    entity_type: String,
    observations: Vec<String>,
}

#[derive(Deserialize)]
struct AddObservationsBody {
    observations: Vec<String>,
}

#[derive(Deserialize)]
struct CreateRelationBody {
    from: String,
    to: String,
    relation_type: String,
}

fn internal_err(e: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

async fn create_entity(
    State(state): State<AppState>,
    Json(body): Json<CreateEntityBody>,
) -> Result<(StatusCode, Json<Entity>), (StatusCode, Json<serde_json::Value>)> {
    let e = Entity {
        id: None,
        name: body.name,
        entity_type: body.entity_type,
        observations: body.observations,
        created_at: surrealdb::types::Datetime::default(),
        updated_at: surrealdb::types::Datetime::default(),
        embedding: None,
    };
    let created = state.storage.create_entity(e).await.map_err(internal_err)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn create_entities_batch(
    State(state): State<AppState>,
    Json(bodies): Json<Vec<CreateEntityBody>>,
) -> Result<Json<Vec<Entity>>, (StatusCode, Json<serde_json::Value>)> {
    let entities = bodies
        .into_iter()
        .map(|b| Entity {
            id: None,
            name: b.name,
            entity_type: b.entity_type,
            observations: b.observations,
            created_at: surrealdb::types::Datetime::default(),
            updated_at: surrealdb::types::Datetime::default(),
            embedding: None,
        })
        .collect();
    let created = state
        .storage
        .create_entities(entities)
        .await
        .map_err(internal_err)?;
    Ok(Json(created))
}

async fn get_graph(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let graph = state.storage.get_graph().await.map_err(internal_err)?;
    Ok(Json(serde_json::json!(graph)))
}

async fn delete_entity(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    state
        .storage
        .delete_entity(&name)
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_observations(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<AddObservationsBody>,
) -> Result<Json<Entity>, (StatusCode, Json<serde_json::Value>)> {
    let updated = state
        .storage
        .add_observations(&name, body.observations)
        .await
        .map_err(internal_err)?;
    Ok(Json(updated))
}

async fn create_relation(
    State(state): State<AppState>,
    Json(body): Json<CreateRelationBody>,
) -> Result<(StatusCode, Json<Relation>), (StatusCode, Json<serde_json::Value>)> {
    let r = Relation {
        id: None,
        from: body.from,
        to: body.to,
        relation_type: body.relation_type,
        created_at: surrealdb::types::Datetime::default(),
    };
    let created = state
        .storage
        .create_relation(r)
        .await
        .map_err(internal_err)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn create_relations_batch(
    State(state): State<AppState>,
    Json(bodies): Json<Vec<CreateRelationBody>>,
) -> Result<Json<Vec<Relation>>, (StatusCode, Json<serde_json::Value>)> {
    let relations = bodies
        .into_iter()
        .map(|b| Relation {
            id: None,
            from: b.from,
            to: b.to,
            relation_type: b.relation_type,
            created_at: surrealdb::types::Datetime::default(),
        })
        .collect();
    let created = state
        .storage
        .create_relations(relations)
        .await
        .map_err(internal_err)?;
    Ok(Json(created))
}

async fn search_entities(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let query = q.q.unwrap_or_default();
    let results = state
        .storage
        .search_entities(&query)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!(results)))
}

// ── Phase 3: Graph-RAG traversal routes ────────────────────────────────

#[derive(Deserialize)]
struct NeighborsQuery {
    depth: Option<u8>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct RelatedQuery {
    relation_type: Option<String>,
    direction: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct PathQuery {
    max_depth: Option<u8>,
}

/// `GET /api/v1/entities/:name/neighbors?depth=2&limit=50`
/// Returns the N-hop subgraph around the given entity.
async fn expand_neighbors(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<NeighborsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let graph = state
        .storage
        .expand_neighbors(&name, q.depth.unwrap_or(2), q.limit.unwrap_or(50))
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!(graph)))
}

/// `GET /api/v1/entities/:name/related?relation_type=WORKS_AT&direction=out`
/// Returns entities related to the given entity.
async fn get_related(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<RelatedQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let entities = state
        .storage
        .get_related(
            &name,
            q.relation_type.as_deref(),
            q.direction.as_deref().unwrap_or("both"),
            q.limit.unwrap_or(20),
        )
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!(entities)))
}

/// `GET /api/v1/entities/:name/path/:to?max_depth=4`
/// Finds shortest paths from `:name` to `:to` via the relation graph.
async fn find_path(
    State(state): State<AppState>,
    Path((from, to)): Path<(String, String)>,
    Query(q): Query<PathQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let paths = state
        .storage
        .find_path(&from, &to, q.max_depth.unwrap_or(4))
        .await
        .map_err(internal_err)?;
    Ok(Json(
        serde_json::json!({ "paths": paths, "from": from, "to": to }),
    ))
}
