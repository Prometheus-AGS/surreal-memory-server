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
async fn taskstream_api_server_mode_full_lifecycle_round_trip() {
    let client = Client::new();
    let base = api_base();
    let suffix = Uuid::new_v4().simple().to_string();
    let name = format!("taskstream-api-{suffix}");
    let user_id = format!("api-user-{suffix}");
    let agent_id = format!("api-agent-{suffix}");

    let create_response = client
        .post(format!("{base}/api/v1/taskstreams"))
        .json(&serde_json::json!({
            "name": name,
            "description": "server api regression",
            "user_id": user_id,
            "agent_id": agent_id,
            "model_id": "gpt-4o",
            "auto_summarize": false
        }))
        .send()
        .await
        .expect("create taskstream request");
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created = read_json(create_response).await;
    assert_eq!(created["name"], name);
    assert_eq!(created["model_id"], "gpt-4o");
    assert_eq!(created["auto_summarize"], false);

    let list_response = client
        .get(format!(
            "{base}/api/v1/taskstreams?user_id={user_id}&agent_id={agent_id}"
        ))
        .send()
        .await
        .expect("list taskstreams request");
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let listed = read_json(list_response).await;
    let streams = listed.as_array().expect("task stream list should be an array");
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0]["name"], name);

    let get_response = client
        .get(format!("{base}/api/v1/taskstreams/{name}"))
        .send()
        .await
        .expect("get taskstream request");
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let fetched = read_json(get_response).await;
    assert_eq!(fetched["name"], name);
    assert_eq!(fetched["model_id"], "gpt-4o");
    assert_eq!(fetched["auto_summarize"], false);

    let add_memory_response = client
        .post(format!("{base}/api/v1/taskstreams/{name}/memories"))
        .json(&serde_json::json!({
            "content": "first server-mode step",
            "user_id": user_id,
            "agent_id": agent_id,
            "categories": ["regression", "server"]
        }))
        .send()
        .await
        .expect("add memory request");
    assert_eq!(add_memory_response.status(), reqwest::StatusCode::CREATED);
    let stored_memory = read_json(add_memory_response).await;
    assert_eq!(stored_memory["content"], "first server-mode step");

    let context_response = client
        .get(format!(
            "{base}/api/v1/taskstreams/{name}/context?model_name=gpt-4o&max_tokens=64"
        ))
        .send()
        .await
        .expect("get context request");
    assert_eq!(context_response.status(), reqwest::StatusCode::OK);
    let context = read_json(context_response).await;
    assert_eq!(context["model_name"], "gpt-4o");
    assert_eq!(context["memories"].as_array().expect("context memories").len(), 1);

    let pause_response = client
        .post(format!("{base}/api/v1/taskstreams/{name}/pause"))
        .send()
        .await
        .expect("pause request");
    assert_eq!(pause_response.status(), reqwest::StatusCode::OK);
    let paused = read_json(pause_response).await;
    assert_eq!(paused["status"], "paused");

    let archive_response = client
        .post(format!("{base}/api/v1/taskstreams/{name}/archive"))
        .send()
        .await
        .expect("archive request");
    assert_eq!(archive_response.status(), reqwest::StatusCode::OK);
    let archived = read_json(archive_response).await;
    assert_eq!(archived["status"], "archived");

    let final_get_response = client
        .get(format!("{base}/api/v1/taskstreams/{name}"))
        .send()
        .await
        .expect("final get taskstream request");
    assert_eq!(final_get_response.status(), reqwest::StatusCode::OK);
    let final_taskstream = read_json(final_get_response).await;
    assert_eq!(final_taskstream["status"], "archived");
}
