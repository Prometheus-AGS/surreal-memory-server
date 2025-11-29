use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone)]
pub enum EmbeddingModel {
    // OpenAI models
    TextEmbedding3Small,
    TextEmbedding3Large,
    TextEmbeddingAda002,

    // Cohere models
    EmbedEnglishV3,
    EmbedMultilingualV3,
    EmbedEnglishLightV3,

    // Local models (Hugging Face)
    BgeSmallEnV15,  // BAAI/bge-small-en-v1.5
    BgeBaseEnV15,   // BAAI/bge-base-en-v1.5
    BgeLargeEnV15,  // BAAI/bge-large-en-v1.5
    AllMiniLML6V2,  // sentence-transformers/all-MiniLM-L6-v2
    AllMiniLML12V2, // sentence-transformers/all-MiniLM-L12-v2
}

impl EmbeddingModel {
    pub fn dimensions(&self) -> usize {
        match self {
            // OpenAI
            Self::TextEmbedding3Small | Self::TextEmbeddingAda002 => 1536,
            Self::TextEmbedding3Large => 3072,

            // Cohere
            Self::EmbedEnglishV3 | Self::EmbedMultilingualV3 => 1024,
            Self::EmbedEnglishLightV3 => 384,

            // Local models
            Self::BgeSmallEnV15 => 384,
            Self::BgeBaseEnV15 => 768,
            Self::BgeLargeEnV15 => 1024,
            Self::AllMiniLML6V2 => 384,
            Self::AllMiniLML12V2 => 384,
        }
    }

    pub fn model_id(&self) -> &str {
        match self {
            Self::BgeSmallEnV15 => "BAAI/bge-small-en-v1.5",
            Self::BgeBaseEnV15 => "BAAI/bge-base-en-v1.5",
            Self::BgeLargeEnV15 => "BAAI/bge-large-en-v1.5",
            Self::AllMiniLML6V2 => "sentence-transformers/all-MiniLM-L6-v2",
            Self::AllMiniLML12V2 => "sentence-transformers/all-MiniLM-L12-v2",
            _ => "",
        }
    }
}

#[async_trait]
pub trait EmbeddingService: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Embedding>;
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>>;
    fn dimensions(&self) -> usize;
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
            Ok(Box::new(candle::CandleEmbeddings::new(&model_id, &cache_dir)?))
        }
    }
}
