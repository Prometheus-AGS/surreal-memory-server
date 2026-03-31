use reqwest::Client;
use serde_json::Value;
use uuid::Uuid;

fn api_base() -> String {
    std::env::var("TEST_API_BASE").unwrap_or_else(|_| "http://127.0.0.1:23001".to_string())
}

async fn read_json(response: reqwest::Response) -> Value {
    let status = response.status();
    let text = response.text().await.expect("response body");
    serde_json::from_str(&text).unwrap_or_else(|_| {
        panic!("expected JSON body for status {status}, got: {text}");
    })
}

#[tokio::test]
async fn mindmap_api_server_mode_create_mutate_get_and_list() {
    let client = Client::new();
    let base = api_base();
    let suffix = Uuid::new_v4().simple().to_string();
    let name = format!("mindmap-api-{suffix}");
    let user_id = format!("api-user-{suffix}");
    let agent_id = format!("api-agent-{suffix}");

    let create_response = client
        .post(format!("{base}/api/v1/mindmaps"))
        .json(&serde_json::json!({
            "name": name,
            "root_label": "Root",
            "map_type": "radial",
            "description": "server api regression",
            "user_id": user_id,
            "agent_id": agent_id,
            "task_stream_id": format!("task_stream:{suffix}"),
            "tags": ["persona", "api"]
        }))
        .send()
        .await
        .expect("create mindmap request");
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created = read_json(create_response).await;
    assert_eq!(created["name"], name);
    assert_eq!(created["tags"][0], "persona");
    assert_eq!(created["tags"][1], "api");

    let add_node_response = client
        .post(format!(
            "{base}/api/v1/mindmaps/{name}/nodes?user_id={user_id}&agent_id={agent_id}"
        ))
        .json(&serde_json::json!({
            "node_id": "beliefs",
            "label": "Beliefs",
            "parent_id": "root",
            "metadata": {
                "source": {
                    "kind": "memory",
                    "confidence": 0.9
                }
            }
        }))
        .send()
        .await
        .expect("add node request");
    assert_eq!(add_node_response.status(), reqwest::StatusCode::OK);
    let with_node = read_json(add_node_response).await;
    assert_eq!(with_node["nodes"].as_array().expect("nodes array").len(), 2);

    let add_edge_response = client
        .post(format!(
            "{base}/api/v1/mindmaps/{name}/edges?user_id={user_id}&agent_id={agent_id}"
        ))
        .json(&serde_json::json!({
            "from_id": "root",
            "to_id": "beliefs",
            "label": "contains",
            "directed": true
        }))
        .send()
        .await
        .expect("add edge request");
    assert_eq!(add_edge_response.status(), reqwest::StatusCode::OK);
    let with_edge = read_json(add_edge_response).await;
    assert_eq!(with_edge["edges"].as_array().expect("edges array").len(), 1);

    let get_response = client
        .get(format!(
            "{base}/api/v1/mindmaps/{name}?user_id={user_id}&agent_id={agent_id}"
        ))
        .send()
        .await
        .expect("get mindmap request");
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let fetched = read_json(get_response).await;
    assert_eq!(fetched["name"], name);
    assert_eq!(fetched["edges"].as_array().expect("edges array").len(), 1);
    assert_eq!(
        fetched["nodes"][1]["metadata"]["source"]["kind"],
        serde_json::json!("memory")
    );

    let list_response = client
        .get(format!(
            "{base}/api/v1/mindmaps?user_id={user_id}&agent_id={agent_id}"
        ))
        .send()
        .await
        .expect("list mindmaps request");
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let listed = read_json(list_response).await;
    let maps = listed.as_array().expect("list response should be an array");
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0]["name"], name);
}
