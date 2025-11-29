use super::{Embedding, EmbeddingModel, EmbeddingService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use hf_hub::{Repo, RepoType, api::tokio::Api};
use std::path::PathBuf;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tokio::sync::{Mutex, OnceCell};

/// Inner struct that holds the actual loaded model
struct CandleEmbeddingsInner {
    model: Mutex<BertModel>,
    tokenizer: Tokenizer,
    device: Device,
    dimensions: usize,
}

/// Lazy-loading wrapper for Candle embeddings.
/// The model is only downloaded and loaded on first use, allowing the MCP server
/// to complete initialization quickly without blocking on model download.
pub struct CandleEmbeddings {
    inner: OnceCell<CandleEmbeddingsInner>,
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
        tracing::info!("Expected dimensions: {} (will verify on first use)", expected_dimensions);

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
    async fn ensure_loaded(&self) -> Result<&CandleEmbeddingsInner> {
        self.inner.get_or_try_init(|| async {
            tracing::info!("Loading Candle embeddings model: {}", self.model_id);
            
            // Determine device (CUDA > Metal > CPU)
            let device = Self::get_device()?;
            tracing::info!("Using device: {:?}", device);

            // Download model files
            let (config_path, tokenizer_path, weights_path) =
                Self::download_model(&self.model_id, &self.cache_dir).await?;

            // Load tokenizer
            let tokenizer = Tokenizer::from_file(tokenizer_path)
                .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

            // Load config
            let config: BertConfig = serde_json::from_reader(std::fs::File::open(config_path)?)?;

            let dimensions = config.hidden_size;

            // Load model weights
            let vb = VarBuilder::from_pth(&weights_path, DType::F32, &device)?;
            let model = BertModel::load(vb, &config)?;

            tracing::info!("Model loaded successfully with {} dimensions", dimensions);

            Ok(CandleEmbeddingsInner {
                model: Mutex::new(model),
                tokenizer,
                device,
                dimensions,
            })
        }).await
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

        // Download required files
        let config_path = repo.get("config.json").await?;
        let tokenizer_path = repo.get("tokenizer.json").await?;

        // Try different weight file names
        let weights_path = if let Ok(path) = repo.get("model.safetensors").await {
            path
        } else if let Ok(path) = repo.get("pytorch_model.bin").await {
            path
        } else {
            anyhow::bail!("No compatible model weights found");
        };

        tracing::info!("Model files downloaded successfully");

        Ok((config_path, tokenizer_path, weights_path))
    }

    fn mean_pooling(embeddings: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        // Mean pooling with attention mask
        let mask = attention_mask.unsqueeze(2)?;
        let mask = mask.broadcast_as(embeddings.shape())?;

        let masked_embeddings = embeddings.mul(&mask)?;
        let sum_embeddings = masked_embeddings.sum(1)?;

        let sum_mask = mask.sum(1)?;
        let sum_mask = sum_mask.clamp(1e-9, f32::MAX)?;

        let pooled = sum_embeddings.div(&sum_mask)?;

        Ok(pooled)
    }

    async fn embed_internal(&self, text: &str) -> Result<Embedding> {
        let inner = self.ensure_loaded().await?;
        
        // Tokenize
        let encoding = inner
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        let tokens = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        // Convert to tensors
        let token_ids = Tensor::new(tokens, &inner.device)?.unsqueeze(0)?;
        let attention_mask = Tensor::new(attention_mask, &inner.device)?.unsqueeze(0)?;
        let token_type_ids = Tensor::zeros(token_ids.shape(), DType::I64, &inner.device)?;

        // Run through model
        let model = inner.model.lock().await;
        let embeddings = model.forward(&token_ids, &token_type_ids, Some(&attention_mask))?;
        drop(model); // Release lock

        // Mean pooling
        let pooled = Self::mean_pooling(&embeddings, &attention_mask)?;

        // L2 normalize
        let normalized = Self::l2_normalize(&pooled)?;

        // Convert to Vec<f32>
        let embedding_vec = normalized.squeeze(0)?.to_vec1::<f32>()?;

        Ok(embedding_vec)
    }

    fn l2_normalize(tensor: &Tensor) -> Result<Tensor> {
        let norm = tensor.sqr()?.sum_keepdim(1)?.sqrt()?;
        let norm = norm.clamp(1e-12, f32::MAX)?;
        Ok(tensor.div(&norm)?)
    }
}

#[async_trait]
impl EmbeddingService for CandleEmbeddings {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        self.embed_internal(text).await
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
        // Process in batches for efficiency
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let embedding = self.embed_internal(&text).await?;
            results.push(embedding);
        }

        Ok(results)
    }

    fn dimensions(&self) -> usize {
        // Return actual dimensions if loaded, otherwise return expected
        self.inner
            .get()
            .map(|i| i.dimensions)
            .unwrap_or(self.expected_dimensions)
    }
}
