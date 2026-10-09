use super::{Embedding, EmbeddingService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct OpenAIEmbeddings {
    client: Client,
    api_key: String,
    model: String,
    dimensions: usize,
    cache_namespace: String,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    input: Vec<String>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl OpenAIEmbeddings {
    pub fn new(api_key: String, model: String) -> Self {
        let dimensions = match model.as_str() {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            "text-embedding-ada-002" => 1536,
            _ => 1536,
        };

        let cache_namespace = serde_json::json!([
            "openai", &model,
            "https://api.openai.com/v1/embeddings;dimensions=default", dimensions
        ])
        .to_string();

        Self {
            cache_namespace,
            client: Client::new(),
            api_key,
            model,
            dimensions,
        }
    }
}

#[async_trait]
impl EmbeddingService for OpenAIEmbeddings {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        let embeddings = self.embed_batch(vec![text.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .context("No embedding returned")
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
        let request = EmbeddingRequest {
            input: texts,
            model: self.model.clone(),
            dimensions: None,
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .context("Failed to send embedding request")?;

        let embedding_response: EmbeddingResponse = response
            .json()
            .await
            .context("Failed to parse embedding response")?;

        Ok(embedding_response
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect())
    }

    fn cache_namespace(&self) -> Option<&str> {
        Some(&self.cache_namespace)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}
