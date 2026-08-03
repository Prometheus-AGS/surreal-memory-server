use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(feature = "local-embeddings")]
pub mod candle;
pub mod cohere;
pub mod openai;

pub type Embedding = Vec<f32>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorEventKind {
    Started,
    Progress,
    Completed,
    Exited,
    Nonresponsive,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorEvent {
    pub operation_id: Option<String>,
    pub generation: u64,
    pub progress_seq: u64,
    pub kind: ExecutorEventKind,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutorSnapshot {
    pub generation: u64,
    pub progress_seq: u64,
    pub exit_count: u64,
    pub last_exit: Option<String>,
    pub error: Option<String>,
}

/// A deterministic, model-safe slice of one logical input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingPlanPart {
    pub part_index: usize,
    pub token_start: usize,
    pub token_end: usize,
    pub token_count: usize,
    pub token_hash: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum EmbeddingProvider {
    OpenAI {
        api_key: String,
        model: String,
    },
    Cohere {
        api_key: String,
        model: String,
    },
    Local {
        model_id: String,
        model_path: Option<String>, // Cache directory
    },
    /// FastEmbed via mempalace-core (all-MiniLM-L6-v2, 384 dims).
    /// Only available when compiled with the `palace` feature.
    #[cfg(feature = "palace")]
    Fast,
}

#[async_trait]
pub trait EmbeddingService: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Embedding>;
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>>;
    fn dimensions(&self) -> usize;

    /// Plan model-safe inputs before inference. Remote providers whose model
    /// limits are enforced by their API retain a single logical part. Local
    /// providers override this using their exact tokenizer and model config.
    async fn plan(&self, text: &str) -> Result<Vec<EmbeddingPlanPart>> {
        Ok(vec![EmbeddingPlanPart {
            part_index: 0,
            token_start: 0,
            // A provider which cannot expose its tokenizer must not label a
            // UTF-8 byte offset as an exact token boundary. Zero denotes an
            // unknown range; callers retain the existing estimator fallback.
            token_end: 0,
            token_count: 0,
            token_hash: Sha256::digest(text.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            content: text.to_owned(),
        }])
    }

    async fn plan_for_operation(
        &self,
        _operation_id: &str,
        text: &str,
    ) -> Result<Vec<EmbeddingPlanPart>> {
        self.plan(text).await
    }

    async fn embed_for_operation(
        &self,
        _operation_id: &str,
        _part_index: usize,
        text: &str,
    ) -> Result<Embedding> {
        self.embed(text).await
    }

    fn subscribe_executor_events(&self) -> Option<tokio::sync::broadcast::Receiver<ExecutorEvent>> {
        None
    }

    fn executor_snapshot(&self) -> Option<ExecutorSnapshot> {
        None
    }

    async fn prepare_operation(
        &self,
        _operation_id: &str,
        _previous: &ExecutorSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    fn executor_snapshot_for_operation(&self, _operation_id: &str) -> Option<ExecutorSnapshot> {
        self.executor_snapshot()
    }

    /// Reports whether the provider is ready to serve embeddings without a
    /// cold-load delay. Remote providers (OpenAI, Cohere) are ready as soon as
    /// they are constructed. Local providers that lazily load a model override
    /// this to report the actual load state, so `/health` can distinguish a
    /// warm server from one about to cold-load on first use.
    fn is_ready(&self) -> bool {
        true
    }
}

pub async fn create_embedding_service(
    provider: EmbeddingProvider,
) -> Result<Box<dyn EmbeddingService>> {
    match provider {
        EmbeddingProvider::OpenAI { api_key, model } => {
            Ok(Box::new(openai::OpenAIEmbeddings::new(api_key, model)))
        }
        EmbeddingProvider::Cohere { api_key, model } => {
            Ok(Box::new(cohere::CohereEmbeddings::new(api_key, model)))
        }
        #[cfg(feature = "local-embeddings")]
        EmbeddingProvider::Local {
            model_id,
            model_path,
        } => {
            let cache_dir = model_path.unwrap_or_else(|| {
                dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("rust-memory-mcp")
                    .join("models")
                    .to_string_lossy()
                    .to_string()
            });

            // Note: CandleEmbeddings uses lazy loading - the model is downloaded
            // and loaded on first embed() call, not here. This allows the MCP
            // server to start quickly without blocking on model download.
            Ok(Box::new(candle::CandleEmbeddings::new(
                &model_id, &cache_dir,
            )?))
        }
        #[cfg(not(feature = "local-embeddings"))]
        EmbeddingProvider::Local { .. } => {
            anyhow::bail!(
                "Local embeddings are not available. Rebuild with --features local-embeddings, \
                 or use EMBEDDING_PROVIDER=openai or EMBEDDING_PROVIDER=cohere"
            )
        }
        #[cfg(feature = "palace")]
        EmbeddingProvider::Fast => Ok(Box::new(
            crate::palace::embedding::FastEmbedService::new().await?,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RemoteFixture;

    #[async_trait]
    impl EmbeddingService for RemoteFixture {
        async fn embed(&self, _text: &str) -> Result<Embedding> {
            Ok(vec![1.0])
        }

        async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
            Ok(texts.into_iter().map(|_| vec![1.0]).collect())
        }

        fn dimensions(&self) -> usize {
            1
        }
    }

    #[tokio::test]
    async fn default_plan_does_not_label_utf8_bytes_as_tokens() {
        let part = RemoteFixture.plan("memory 📚").await.unwrap().remove(0);

        assert_eq!(part.token_start, 0);
        assert_eq!(part.token_end, 0);
        assert_eq!(part.token_count, 0);
        assert_eq!(part.content, "memory 📚");
    }
}
