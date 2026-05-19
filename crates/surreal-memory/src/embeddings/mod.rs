use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[cfg(feature = "local-embeddings")]
pub mod candle;
pub mod cohere;
pub mod openai;

pub type Embedding = Vec<f32>;

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
