use crate::embeddings::EmbeddingProvider;
use serde::Deserialize;
use std::{env, path::PathBuf};

pub const DEFAULT_LOCAL_EMBEDDING_MODEL: &str = "BAAI/bge-small-en-v1.5";
pub const DEFAULT_LOCAL_EMBEDDING_REVISION: &str = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalEmbeddingBackend {
    Candle,
    Mlx,
}

impl LocalEmbeddingBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candle => "candle",
            Self::Mlx => "mlx",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub surreal_mode: SurrealMode,
    pub surreal_endpoint: Option<String>,
    pub surreal_namespace: String,
    pub surreal_database: String,
    pub surreal_username: Option<String>,
    pub surreal_password: Option<String>,
    pub embedded_path: Option<String>,
    pub embedding_provider: EmbeddingProvider,
    pub local_embedding_backend: LocalEmbeddingBackend,
    pub local_embedding_executor: Option<PathBuf>,
    pub local_embedding_model_revision: String,
    pub local_embedding_dimensions: usize,
    /// When true, the embedding model is loaded eagerly at startup so the first
    /// user-facing write does not pay the cold-load latency. Defaults to true;
    /// set `EMBEDDING_WARMUP=false` to restore purely lazy loading.
    pub embedding_warmup: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurrealMode {
    Embedded,
    Server,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let mode_str = env::var("SURREAL_MODE").unwrap_or_else(|_| "embedded".to_string());
        let surreal_mode = match mode_str.as_str() {
            "server" => SurrealMode::Server,
            _ => SurrealMode::Embedded,
        };

        // Embedding configuration
        let embedding_provider = match env::var("EMBEDDING_PROVIDER")
            .unwrap_or_else(|_| "local".to_string())
            .as_str()
        {
            "openai" => {
                let api_key = env::var("OPENAI_API_KEY")?;
                let model = env::var("OPENAI_EMBEDDING_MODEL")
                    .unwrap_or_else(|_| "text-embedding-3-small".to_string());
                EmbeddingProvider::OpenAI { api_key, model }
            }
            "cohere" => {
                let api_key = env::var("COHERE_API_KEY")?;
                let model = env::var("COHERE_EMBEDDING_MODEL")
                    .unwrap_or_else(|_| "embed-english-v3.0".to_string());
                EmbeddingProvider::Cohere { api_key, model }
            }
            #[cfg(feature = "palace")]
            "fast" => EmbeddingProvider::Fast,
            _ => {
                // Default to local embeddings
                let model_id = env::var("LOCAL_EMBEDDING_MODEL")
                    .unwrap_or_else(|_| DEFAULT_LOCAL_EMBEDDING_MODEL.to_string());
                let model_path = env::var("MODEL_CACHE_DIR").ok();
                EmbeddingProvider::Local {
                    model_id,
                    model_path,
                }
            }
        };

        let config = Config {
            surreal_mode,
            surreal_endpoint: env::var("SURREAL_ENDPOINT").ok(),
            surreal_namespace: env::var("SURREAL_NAMESPACE")
                .unwrap_or_else(|_| "memory".to_string()),
            surreal_database: env::var("SURREAL_DATABASE").unwrap_or_else(|_| "mcp".to_string()),
            surreal_username: env::var("SURREAL_USERNAME").ok(),
            surreal_password: env::var("SURREAL_PASSWORD").ok(),
            embedded_path: env::var("SURREAL_PATH")
                .ok()
                .or_else(|| Some("./data/memory.db".to_string())),
            embedding_provider,
            local_embedding_backend: match env::var("LOCAL_EMBEDDING_BACKEND")
                .unwrap_or_else(|_| "candle".to_string())
                .to_ascii_lowercase()
                .as_str()
            {
                "candle" => LocalEmbeddingBackend::Candle,
                "mlx" => LocalEmbeddingBackend::Mlx,
                value => anyhow::bail!(
                    "LOCAL_EMBEDDING_BACKEND must be 'candle' or 'mlx', got '{value}'"
                ),
            },
            local_embedding_executor: env::var_os("LOCAL_EMBEDDING_EXECUTOR").map(PathBuf::from),
            local_embedding_model_revision: env::var("LOCAL_EMBEDDING_MODEL_REVISION")
                .unwrap_or_else(|_| DEFAULT_LOCAL_EMBEDDING_REVISION.to_string()),
            local_embedding_dimensions: env::var("LOCAL_EMBEDDING_DIMENSIONS")
                .ok()
                .map(|value| value.parse::<usize>())
                .transpose()
                .map_err(|error| anyhow::anyhow!("invalid LOCAL_EMBEDDING_DIMENSIONS: {error}"))?
                .unwrap_or(384),
            // Default on: with warmup off, the first user-facing request paid
            // the entire cold model load, which is exactly the window in which
            // the supervisor watchdog used to kill the executor. Warmup moves
            // that cost to startup, where a failure only logs and falls back to
            // lazy loading. Opt out with EMBEDDING_WARMUP=false.
            embedding_warmup: env::var("EMBEDDING_WARMUP")
                .map(|v| !(v.eq_ignore_ascii_case("false") || v == "0"))
                .unwrap_or(true),
        };

        Ok(config)
    }
}
