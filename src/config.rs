use crate::embeddings::EmbeddingProvider;
use serde::Deserialize;
use std::env;

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
                    .unwrap_or_else(|_| "BAAI/bge-small-en-v1.5".to_string());
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
