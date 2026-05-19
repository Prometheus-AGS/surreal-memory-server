use super::{Embedding, EmbeddingService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use hf_hub::{Repo, RepoType, api::tokio::Api};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokenizers::Tokenizer;
use tokio::sync::{Mutex, OnceCell};

/// Upper bound on a cold-cache HuggingFace model download. Exceeding it yields a
/// typed error instead of an open-ended hang on the write path.
const MODEL_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Inner struct that holds the actual loaded model
/// The mutex protects all GPU operations to prevent Metal command buffer conflicts
struct CandleEmbeddingsInner {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    #[allow(dead_code)] // Kept for potential future use in dimension validation
    dimensions: usize,
}

/// Thread-safe wrapper that serializes all GPU operations.
///
/// The inner state is held in `Arc<Mutex<_>>` so it can be moved into a
/// `spawn_blocking` task: the CPU-heavy tokenize + forward pass runs on a
/// blocking thread (acquiring the mutex there) and never starves the runtime.
pub struct CandleEmbeddings {
    inner: OnceCell<Arc<Mutex<CandleEmbeddingsInner>>>,
    model_id: String,
    cache_dir: String,
    expected_dimensions: usize,
}

impl CandleEmbeddings {
    /// Creates a new lazy-loading Candle embeddings instance.
    /// The model is NOT downloaded or loaded here - it will be loaded on first use.
    /// This allows the MCP server to start immediately without blocking on model download.
    pub fn new(model_id: &str, cache_dir: &str) -> Result<Self> {
        tracing::info!("Preparing lazy Candle embeddings for model: {}", model_id);

        // Estimate dimensions based on known models
        let expected_dimensions = Self::estimate_dimensions(model_id);
        tracing::info!(
            "Expected dimensions: {} (will verify on first use)",
            expected_dimensions
        );

        Ok(Self {
            inner: OnceCell::new(),
            model_id: model_id.to_string(),
            cache_dir: cache_dir.to_string(),
            expected_dimensions,
        })
    }

    /// Estimate dimensions based on known model IDs
    fn estimate_dimensions(model_id: &str) -> usize {
        match model_id {
            id if id.contains("bge-small") => 384,
            id if id.contains("bge-base") => 768,
            id if id.contains("bge-large") => 1024,
            id if id.contains("MiniLM-L6") => 384,
            id if id.contains("MiniLM-L12") => 384,
            id if id.contains("all-mpnet-base") => 768,
            _ => 384, // Default fallback
        }
    }

    /// Ensures the model is loaded, downloading if necessary.
    /// This is called lazily on first embed request.
    ///
    /// The HuggingFace download is bounded by `MODEL_DOWNLOAD_TIMEOUT` so a cold
    /// cache or unreachable network fails fast with a typed error rather than
    /// hanging the write path. The CPU-heavy model build runs on a blocking
    /// thread (`spawn_blocking`) so it never starves the async runtime.
    async fn ensure_loaded(&self) -> Result<&Arc<Mutex<CandleEmbeddingsInner>>> {
        self.inner
            .get_or_try_init(|| async {
                tracing::info!("Loading Candle embeddings model: {}", self.model_id);

                // Determine device (CUDA > Metal > CPU)
                let device = Self::get_device().context("Failed to get compute device")?;
                tracing::info!("Using device: {:?}", device);

                // Download model files — bounded so a cold cache or network
                // failure surfaces as a clear error instead of an open hang.
                let (config_path, tokenizer_path, weights_path) = tokio::time::timeout(
                    MODEL_DOWNLOAD_TIMEOUT,
                    Self::download_model(&self.model_id, &self.cache_dir),
                )
                .await
                .with_context(|| {
                    format!(
                        "Embedding model download exceeded {}s timeout for '{}'",
                        MODEL_DOWNLOAD_TIMEOUT.as_secs(),
                        self.model_id
                    )
                })?
                .context("Failed to download model files")?;

                tracing::debug!("Config path: {:?}", config_path);
                tracing::debug!("Tokenizer path: {:?}", tokenizer_path);
                tracing::debug!("Weights path: {:?}", weights_path);

                // The model build is synchronous, CPU-heavy work (file reads,
                // weight parsing, tensor allocation). Run it on a blocking
                // thread so it does not starve tokio runtime workers.
                let inner = tokio::task::spawn_blocking(move || {
                    Self::build_model(config_path, tokenizer_path, weights_path, device)
                })
                .await
                .context("Embedding model build task panicked")??;

                tracing::info!(
                    "Model loaded successfully with {} dimensions",
                    inner.dimensions
                );

                Ok(Arc::new(Mutex::new(inner)))
            })
            .await
    }

    /// Loads the model eagerly. Used by an optional startup warmup so the
    /// first user-facing write does not pay the cold-load cost.
    pub async fn warmup(&self) -> Result<()> {
        self.ensure_loaded().await?;
        Ok(())
    }

    /// Reports whether the model has been loaded and verified. Lets `/health`
    /// distinguish "process up" from "embedding model ready".
    pub fn is_loaded(&self) -> bool {
        self.inner.get().is_some()
    }

    /// Synchronous model build: load tokenizer, config, and weights, then
    /// construct the BERT model. Runs inside `spawn_blocking`.
    fn build_model(
        config_path: PathBuf,
        tokenizer_path: PathBuf,
        weights_path: PathBuf,
        device: Device,
    ) -> Result<CandleEmbeddingsInner> {
        tracing::info!("Loading tokenizer...");
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            anyhow::anyhow!("Failed to load tokenizer from {:?}: {}", tokenizer_path, e)
        })?;

        tracing::info!("Loading config...");
        let config: BertConfig = serde_json::from_reader(
            std::fs::File::open(&config_path)
                .context(format!("Failed to open config file: {:?}", config_path))?,
        )
        .context("Failed to parse config.json")?;

        let dimensions = config.hidden_size;
        tracing::info!("Model config loaded: {} dimensions", dimensions);

        tracing::info!("Loading model weights from: {:?}", weights_path);
        let weights_path_str = weights_path.to_string_lossy();

        let vb = if weights_path_str.ends_with(".safetensors") {
            tracing::info!("Loading SafeTensors weights...");
            // SAFETY: `weights_path` points to a file just downloaded from the
            // HuggingFace Hub into a process-owned cache directory. It is a
            // valid safetensors file and is not concurrently modified for the
            // lifetime of the memory map.
            unsafe {
                VarBuilder::from_mmaped_safetensors(&[&weights_path], DType::F32, &device)
                    .context("Failed to load SafeTensors weights")?
            }
        } else {
            tracing::info!("Loading PyTorch weights...");
            VarBuilder::from_pth(&weights_path, DType::F32, &device)
                .context("Failed to load PyTorch weights")?
        };

        tracing::info!("Building BERT model...");
        let model =
            BertModel::load(vb, &config).context("Failed to build BERT model from weights")?;

        Ok(CandleEmbeddingsInner {
            model,
            tokenizer,
            device,
            dimensions,
        })
    }

    fn get_device() -> Result<Device> {
        #[cfg(feature = "cuda")]
        {
            if candle_core::utils::cuda_is_available() {
                tracing::info!("CUDA available, using GPU");
                return Ok(Device::new_cuda(0)?);
            }
        }

        #[cfg(feature = "metal")]
        {
            if candle_core::utils::metal_is_available() {
                tracing::info!("Metal available, using GPU");
                return Ok(Device::new_metal(0)?);
            }
        }

        tracing::info!("Using CPU");
        Ok(Device::Cpu)
    }

    async fn download_model(
        model_id: &str,
        cache_dir: &str,
    ) -> Result<(PathBuf, PathBuf, PathBuf)> {
        tracing::info!("Downloading model from Hugging Face: {}", model_id);

        let api = Api::new()?;
        let repo = api.repo(Repo::new(model_id.to_string(), RepoType::Model));

        // Set cache directory
        std::fs::create_dir_all(cache_dir)?;

        // Download config, tokenizer, and weights concurrently. The weights
        // future tries safetensors first and falls back to the PyTorch file.
        let config_fut = repo.get("config.json");
        let tokenizer_fut = repo.get("tokenizer.json");
        let weights_fut = async {
            match repo.get("model.safetensors").await {
                Ok(path) => Ok(path),
                Err(_) => repo
                    .get("pytorch_model.bin")
                    .await
                    .map_err(|_| anyhow::anyhow!("No compatible model weights found")),
            }
        };

        let (config_path, tokenizer_path, weights_path) = tokio::try_join!(
            async { config_fut.await.map_err(anyhow::Error::from) },
            async { tokenizer_fut.await.map_err(anyhow::Error::from) },
            weights_fut
        )?;

        tracing::info!("Model files downloaded successfully");

        Ok((config_path, tokenizer_path, weights_path))
    }

    fn mean_pooling(embeddings: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        // Mean pooling with attention mask. Each step carries error context so a
        // failure is diagnosable without per-step logging on the hot path.
        let mask_f32 = attention_mask
            .to_dtype(DType::F32)
            .context("Failed to cast attention_mask to F32")?;
        let mask = mask_f32.unsqueeze(2).context("Failed to unsqueeze mask")?;
        let mask = mask
            .broadcast_as(embeddings.shape())
            .context("Failed to broadcast mask")?;

        let masked_embeddings = embeddings
            .mul(&mask)
            .context("Failed to multiply embeddings by mask")?;
        let sum_embeddings = masked_embeddings
            .sum(1)
            .context("Failed to sum embeddings")?;
        let sum_mask = mask.sum(1).context("Failed to sum mask")?;
        let sum_mask = sum_mask
            .clamp(1e-9, f32::MAX)
            .context("Failed to clamp sum_mask")?;

        sum_embeddings
            .div(&sum_mask)
            .context("Failed to divide for mean pooling")
    }

    async fn embed_internal(&self, text: &str) -> Result<Embedding> {
        tracing::debug!("embed_internal called for text length: {}", text.len());

        let inner_mutex = self
            .ensure_loaded()
            .await
            .map_err(|e| e.context("Failed to load embedding model"))?;

        // The tokenize + forward pass is synchronous, CPU/GPU-heavy work. Run it
        // on a blocking thread so it never starves the async runtime. The mutex
        // (acquired inside the blocking thread via `blocking_lock`) still
        // serializes GPU access to avoid Metal command-buffer conflicts.
        let inner_mutex = Arc::clone(inner_mutex);
        let text = text.to_string();

        tokio::task::spawn_blocking(move || {
            let inner = inner_mutex.blocking_lock();
            Self::compute_embedding(&inner, &text)
        })
        .await
        .context("Embedding compute task panicked")?
    }

    /// Synchronous embedding compute: tokenize, forward pass, mean pooling, and
    /// L2 normalization. Runs inside `spawn_blocking` while holding the inner
    /// mutex, so GPU command submission stays serialized.
    fn compute_embedding(inner: &CandleEmbeddingsInner, text: &str) -> Result<Embedding> {
        let encoding = inner
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        let tokens = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();
        tracing::debug!("Tokenized into {} tokens", tokens.len());

        let token_ids = Tensor::new(tokens, &inner.device)
            .context("Failed to create token_ids tensor")?
            .unsqueeze(0)
            .context("Failed to unsqueeze token_ids")?;

        let attention_mask_tensor = Tensor::new(attention_mask, &inner.device)
            .context("Failed to create attention_mask tensor")?
            .unsqueeze(0)
            .context("Failed to unsqueeze attention_mask")?;

        let token_type_ids = Tensor::zeros(token_ids.shape(), DType::I64, &inner.device)
            .context("Failed to create token_type_ids tensor")?;

        let embeddings = inner
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask_tensor))
            .map_err(|e| anyhow::anyhow!("Model forward pass failed: {}", e))?;

        let pooled = Self::mean_pooling(&embeddings, &attention_mask_tensor)
            .context("Mean pooling failed")?;

        let normalized = Self::l2_normalize(&pooled).context("L2 normalization failed")?;

        let embedding_vec = normalized
            .squeeze(0)
            .context("Failed to squeeze output")?
            .to_vec1::<f32>()
            .context("Failed to convert to Vec<f32>")?;

        tracing::debug!(
            "Successfully generated embedding with {} dimensions",
            embedding_vec.len()
        );
        Ok(embedding_vec)
    }

    fn l2_normalize(tensor: &Tensor) -> Result<Tensor> {
        // tensor shape is [1, 384]
        // We need to compute L2 norm across dimension 1 and normalize
        let squared = tensor.sqr().context("Failed to square tensor")?;

        let sum_squared = squared
            .sum_keepdim(1)
            .context("Failed to sum squared values")?;

        let norm = sum_squared.sqrt().context("Failed to compute sqrt")?;

        // norm shape is [1, 1], clamp to avoid division by zero
        let norm_clamped = norm
            .clamp(1e-12, f32::MAX)
            .context("Failed to clamp norm")?;

        // Use broadcast_div which handles the broadcasting automatically
        // This divides [1, 384] by [1, 1] with proper broadcasting
        tensor
            .broadcast_div(&norm_clamped)
            .context("Failed to divide by norm")
    }
}

#[async_trait]
impl EmbeddingService for CandleEmbeddings {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        self.embed_internal(text).await
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
        // Process sequentially to avoid Metal command buffer conflicts
        // Each embed_internal call acquires the mutex, ensuring serialized GPU access
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let embedding = self.embed_internal(&text).await?;
            results.push(embedding);
        }

        Ok(results)
    }

    fn dimensions(&self) -> usize {
        // Return expected dimensions (we can't easily access the mutex synchronously)
        self.expected_dimensions
    }

    fn is_ready(&self) -> bool {
        self.is_loaded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_does_not_load_model_eagerly() {
        let embedder = CandleEmbeddings::new("BAAI/bge-small-en-v1.5", "/tmp/test-cache")
            .expect("construct lazy embedder");
        assert!(
            !embedder.is_loaded(),
            "model must not be loaded until first use or warmup"
        );
        assert_eq!(embedder.dimensions(), 384);
    }

    #[test]
    fn download_timeout_is_bounded() {
        // A cold-cache download must be bounded so the write path cannot hang
        // open-endedly. The bound is enforced via tokio::time::timeout in
        // ensure_loaded; this guards against the constant being removed.
        assert!(MODEL_DOWNLOAD_TIMEOUT.as_secs() > 0);
        assert!(MODEL_DOWNLOAD_TIMEOUT.as_secs() <= 600);
    }

    #[tokio::test]
    async fn warmup_with_unresolvable_model_fails_bounded() {
        // An unresolvable model id must surface a typed error within the
        // download bound — never an open-ended hang.
        let embedder = CandleEmbeddings::new(
            "surreal-memory-test/definitely-not-a-real-model",
            &std::env::temp_dir()
                .join("candle-warmup-test")
                .to_string_lossy(),
        )
        .expect("construct lazy embedder");

        let result = tokio::time::timeout(
            MODEL_DOWNLOAD_TIMEOUT + Duration::from_secs(5),
            embedder.warmup(),
        )
        .await;

        assert!(
            result.is_ok(),
            "warmup must return within the bounded download timeout"
        );
        assert!(
            result.unwrap().is_err(),
            "warmup with an unresolvable model must return a typed error"
        );
        assert!(!embedder.is_loaded(), "failed warmup must not mark loaded");
    }

    fn estimate(model_id: &str) -> usize {
        CandleEmbeddings::estimate_dimensions(model_id)
    }

    #[test]
    fn dimension_estimation_covers_known_models() {
        assert_eq!(estimate("BAAI/bge-small-en-v1.5"), 384);
        assert_eq!(estimate("BAAI/bge-base-en-v1.5"), 768);
        assert_eq!(estimate("BAAI/bge-large-en-v1.5"), 1024);
        assert_eq!(estimate("sentence-transformers/all-MiniLM-L6-v2"), 384);
    }
}
