use super::{Embedding, EmbeddingService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct CohereEmbeddings {
    client: Client,
    api_key: String,
    model: String,
    dimensions: usize,
}

#[derive(Serialize)]
struct CohereRequest {
    texts: Vec<String>,
    model: String,
    input_type: String,
    embedding_types: Vec<String>,
}

#[derive(Deserialize)]
struct CohereResponse {
    embeddings: CohereEmbeddingPayload,
}

#[derive(Deserialize)]
struct CohereEmbeddingPayload {
    float: Vec<Vec<f32>>,
}

impl CohereEmbeddings {
    pub fn new(api_key: String, model: String) -> Self {
        let dimensions = match model.as_str() {
            "embed-english-v3.0" | "embed-multilingual-v3.0" => 1024,
            "embed-english-light-v3.0" | "embed-multilingual-light-v3.0" => 384,
            _ => 1024,
        };

        Self {
            client: Client::new(),
            api_key,
            model,
            dimensions,
        }
    }
}

#[async_trait]
impl EmbeddingService for CohereEmbeddings {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        let embeddings = self.embed_batch(vec![text.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .context("No embedding returned")
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
        let request = CohereRequest {
            texts,
            model: self.model.clone(),
            input_type: "search_document".to_string(),
            embedding_types: vec!["float".to_string()],
        };

        let response = self
            .client
            .post("https://api.cohere.ai/v1/embed")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send Cohere embedding request")?;

        let cohere_response: CohereResponse = response
            .json()
            .await
            .context("Failed to parse Cohere response")?;

        Ok(cohere_response.embeddings.float)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}
