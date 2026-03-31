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
async fn taskstream_api_server_mode_create_and_get_round_trip() {
    let client = Client::new();
    let base = api_base();
    let name = format!("taskstream-api-{}", Uuid::new_v4().simple());

    let create_response = client
        .post(format!("{base}/api/v1/taskstreams"))
        .json(&serde_json::json!({
            "name": name,
            "description": "server api regression",
            "user_id": "api-user",
            "agent_id": "api-agent",
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
}
