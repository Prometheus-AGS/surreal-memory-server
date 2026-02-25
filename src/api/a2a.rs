//! A2A (Agent-to-Agent) API routes.
//! GET  /agent.json       — A2A Agent Card
//! POST /a2a/tasks/send   — create and start a task
//! GET  /a2a/tasks/:id    — get task status

use super::AppState;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/agent.json", get(agent_card))
        .route("/a2a/tasks/send", post(send_task))
        .route("/a2a/tasks/{id}", get(get_task))
}

/// A2A Agent Card — describes capabilities per the A2A spec.
async fn agent_card() -> Json<serde_json::Value> {
    Json(json!({
        "schema_version": "0.3",
        "name": "surreal-memory",
        "description": "SurrealDB-backed agent memory server with scoped memory, knowledge graphs, TaskStreams, and Mindmaps. mem0-compatible API.",
        "url": "http://localhost:3001",
        "version": env!("CARGO_PKG_VERSION"),
        "capabilities": {
            "streaming": false,
            "push_notifications": false,
            "state_transition_history": true
        },
        "authentication": {
            "schemes": []
        },
        "skills": [
            {
                "id": "memory",
                "name": "Scoped Memory",
                "description": "mem0-compatible memory add/search/get/update/delete with user/agent/session scoping and semantic deduplication.",
                "tags": ["memory", "scoped", "mem0"],
                "examples": ["Remember that the user prefers TypeScript", "What did the user say about their project?"]
            },
            {
                "id": "knowledge_graph",
                "name": "Knowledge Graph",
                "description": "Entity/relation knowledge graph with semantic search and batch operations.",
                "tags": ["knowledge", "graph", "entities", "relations"]
            },
            {
                "id": "task_streams",
                "name": "TaskStreams",
                "description": "Named long-running task memory streams with model-aware token budgeting.",
                "tags": ["tasks", "context", "streaming"]
            },
            {
                "id": "mindmaps",
                "name": "Mindmaps",
                "description": "Structured visual knowledge maps (radial, concept, argument, tree, temporal) for persona modeling and ideation.",
                "tags": ["mindmap", "persona", "ideation", "visualization"]
            }
        ]
    }))
}

// ── Minimal A2A Task scaffolding ──────────────────────────────────────────────
// Full A2A implementation (SSE streaming, task state machine) would follow the
// complete A2A spec. This implementation provides the HTTP scaffolding.

#[derive(Deserialize)]
struct SendTaskRequest {
    skill_id: Option<String>,
    input: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct TaskResponse {
    id: String,
    status: String,
    skill_id: Option<String>,
    result: Option<serde_json::Value>,
}

async fn send_task(
    State(_state): State<AppState>,
    Json(req): Json<SendTaskRequest>,
) -> Result<(StatusCode, Json<TaskResponse>), (StatusCode, Json<serde_json::Value>)> {
    // Generate a task id — in a full implementation this would be persisted
    let task_id = uuid_v4();
    let resp = TaskResponse {
        id: task_id,
        status: "submitted".to_string(),
        skill_id: req.skill_id,
        result: Some(json!({
            "message": "A2A task routing is not yet implemented. Use MCP tools or REST API directly.",
            "received_input": req.input
        })),
    };
    Ok((StatusCode::ACCEPTED, Json(resp)))
}

async fn get_task(Path(id): Path<String>) -> Json<serde_json::Value> {
    // Stub — full implementation would look up task state from storage
    Json(json!({
        "id": id,
        "status": "unknown",
        "message": "Task persistence not yet implemented in this version."
    }))
}

fn uuid_v4() -> String {
    // Simple UUID v4 without adding the uuid crate
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:016x}-{:04x}", t.as_nanos(), t.subsec_micros())
}
