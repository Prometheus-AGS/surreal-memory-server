use super::{Embedding, EmbeddingService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use hf_hub::{Repo, RepoType, api::tokio::Api};
use std::path::PathBuf;
use tokenizers::Tokenizer;
use tokio::sync::{Mutex, OnceCell};

/// Inner struct that holds the actual loaded model
/// The mutex protects all GPU operations to prevent Metal command buffer conflicts
struct CandleEmbeddingsInner {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    #[allow(dead_code)] // Kept for potential future use in dimension validation
    dimensions: usize,
}

/// Thread-safe wrapper that serializes all GPU operations
pub struct CandleEmbeddings {
    inner: OnceCell<Mutex<CandleEmbeddingsInner>>,
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
    async fn ensure_loaded(&self) -> Result<&Mutex<CandleEmbeddingsInner>> {
        self.inner
            .get_or_try_init(|| async {
                tracing::info!("Loading Candle embeddings model: {}", self.model_id);

                // Determine device (CUDA > Metal > CPU)
                let device = Self::get_device().context("Failed to get compute device")?;
                tracing::info!("Using device: {:?}", device);

                // Download model files
                let (config_path, tokenizer_path, weights_path) =
                    Self::download_model(&self.model_id, &self.cache_dir)
                        .await
                        .context("Failed to download model files")?;

                tracing::debug!("Config path: {:?}", config_path);
                tracing::debug!("Tokenizer path: {:?}", tokenizer_path);
                tracing::debug!("Weights path: {:?}", weights_path);

                // Load tokenizer
                tracing::info!("Loading tokenizer...");
                let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
                    anyhow::anyhow!("Failed to load tokenizer from {:?}: {}", tokenizer_path, e)
                })?;

                // Load config
                tracing::info!("Loading config...");
                let config: BertConfig = serde_json::from_reader(
                    std::fs::File::open(&config_path)
                        .context(format!("Failed to open config file: {:?}", config_path))?,
                )
                .context("Failed to parse config.json")?;

                let dimensions = config.hidden_size;
                tracing::info!("Model config loaded: {} dimensions", dimensions);

                // Load model weights - try safetensors first, then pytorch
                tracing::info!("Loading model weights from: {:?}", weights_path);
                let weights_path_str = weights_path.to_string_lossy();

                let vb = if weights_path_str.ends_with(".safetensors") {
                    tracing::info!("Loading SafeTensors weights...");
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
                let model = BertModel::load(vb, &config)
                    .context("Failed to build BERT model from weights")?;

                tracing::info!("Model loaded successfully with {} dimensions", dimensions);

                Ok(Mutex::new(CandleEmbeddingsInner {
                    model,
                    tokenizer,
                    device,
                    dimensions,
                }))
            })
            .await
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
        tracing::info!(
            "mean_pooling: embeddings shape={:?}, dtype={:?}, device={:?}",
            embeddings.shape(),
            embeddings.dtype(),
            embeddings.device()
        );
        tracing::info!(
            "mean_pooling: attention_mask shape={:?}, dtype={:?}, device={:?}",
            attention_mask.shape(),
            attention_mask.dtype(),
            attention_mask.device()
        );

        // Need to cast attention_mask to f32 for multiplication
        tracing::info!("mean_pooling: casting attention_mask to F32...");
        let mask_f32 = attention_mask
            .to_dtype(DType::F32)
            .context("Failed to cast attention_mask to F32")?;
        tracing::info!(
            "mean_pooling: cast successful, shape={:?}",
            mask_f32.shape()
        );

        tracing::info!("mean_pooling: unsqueezing mask...");
        let mask = mask_f32.unsqueeze(2).context("Failed to unsqueeze mask")?;
        tracing::info!(
            "mean_pooling: unsqueeze successful, shape={:?}",
            mask.shape()
        );

        tracing::info!("mean_pooling: broadcasting mask to embeddings shape...");
        let mask = mask
            .broadcast_as(embeddings.shape())
            .context("Failed to broadcast mask")?;
        tracing::info!(
            "mean_pooling: broadcast successful, shape={:?}",
            mask.shape()
        );

        tracing::info!("mean_pooling: multiplying embeddings by mask...");
        let masked_embeddings = embeddings
            .mul(&mask)
            .context("Failed to multiply embeddings by mask")?;
        tracing::info!(
            "mean_pooling: multiplication successful, shape={:?}",
            masked_embeddings.shape()
        );

        tracing::info!("mean_pooling: summing embeddings along axis 1...");
        let sum_embeddings = masked_embeddings
            .sum(1)
            .context("Failed to sum embeddings")?;
        tracing::info!(
            "mean_pooling: sum_embeddings successful, shape={:?}",
            sum_embeddings.shape()
        );

        tracing::info!("mean_pooling: summing mask along axis 1...");
        let sum_mask = mask.sum(1).context("Failed to sum mask")?;
        tracing::info!(
            "mean_pooling: sum_mask successful, shape={:?}",
            sum_mask.shape()
        );

        tracing::info!("mean_pooling: clamping sum_mask...");
        let sum_mask = sum_mask
            .clamp(1e-9, f32::MAX)
            .context("Failed to clamp sum_mask")?;
        tracing::info!("mean_pooling: clamp successful");

        tracing::info!("mean_pooling: dividing for final pooled output...");
        let pooled = sum_embeddings
            .div(&sum_mask)
            .context("Failed to divide for mean pooling")?;
        tracing::info!(
            "mean_pooling: division successful, final shape={:?}",
            pooled.shape()
        );

        Ok(pooled)
    }

    async fn embed_internal(&self, text: &str) -> Result<Embedding> {
        tracing::info!("embed_internal called for text length: {}", text.len());

        let inner_mutex = match self.ensure_loaded().await {
            Ok(inner) => {
                tracing::info!("Model loaded successfully");
                inner
            }
            Err(e) => {
                tracing::error!("Failed to load embedding model: {:?}", e);
                return Err(e.context("Failed to load embedding model"));
            }
        };

        // Lock the mutex for ALL GPU operations to prevent Metal command buffer conflicts
        // This ensures only one thread is submitting commands to the Metal GPU at a time
        let inner = inner_mutex.lock().await;

        // Tokenize (CPU operation, but we keep it in the lock for simplicity)
        tracing::info!("Tokenizing text: {}...", &text[..text.len().min(50)]);
        let encoding = match inner.tokenizer.encode(text, true) {
            Ok(enc) => {
                tracing::info!("Tokenization successful");
                enc
            }
            Err(e) => {
                tracing::error!("Tokenization failed: {}", e);
                return Err(anyhow::anyhow!("Tokenization failed: {}", e));
            }
        };

        let tokens = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();
        tracing::info!("Tokenized into {} tokens", tokens.len());

        // Convert to tensors
        tracing::info!("Creating tensors on device: {:?}", inner.device);
        let token_ids = Tensor::new(tokens, &inner.device)
            .context("Failed to create token_ids tensor")?
            .unsqueeze(0)
            .context("Failed to unsqueeze token_ids")?;
        tracing::info!("token_ids tensor created: {:?}", token_ids.shape());

        let attention_mask_tensor = Tensor::new(attention_mask, &inner.device)
            .context("Failed to create attention_mask tensor")?
            .unsqueeze(0)
            .context("Failed to unsqueeze attention_mask")?;
        tracing::info!(
            "attention_mask tensor created: {:?}",
            attention_mask_tensor.shape()
        );

        let token_type_ids = Tensor::zeros(token_ids.shape(), DType::I64, &inner.device)
            .context("Failed to create token_type_ids tensor")?;
        tracing::info!(
            "token_type_ids tensor created: {:?}",
            token_type_ids.shape()
        );

        // Run through model
        tracing::info!("Running model forward pass...");
        let embeddings =
            match inner
                .model
                .forward(&token_ids, &token_type_ids, Some(&attention_mask_tensor))
            {
                Ok(emb) => {
                    tracing::info!("Forward pass successful, output shape: {:?}", emb.shape());
                    emb
                }
                Err(e) => {
                    tracing::error!("Model forward pass failed: {:?}", e);
                    return Err(anyhow::anyhow!("Model forward pass failed: {}", e));
                }
            };

        // Mean pooling (still on GPU, within mutex scope)
        tracing::info!("Applying mean pooling...");
        let pooled = match Self::mean_pooling(&embeddings, &attention_mask_tensor) {
            Ok(p) => {
                tracing::info!("Mean pooling successful, shape: {:?}", p.shape());
                p
            }
            Err(e) => {
                tracing::error!("Mean pooling failed: {:?}", e);
                return Err(e.context("Mean pooling failed"));
            }
        };

        // L2 normalize (still on GPU, within mutex scope)
        tracing::info!("Applying L2 normalization...");
        let normalized = match Self::l2_normalize(&pooled) {
            Ok(n) => {
                tracing::info!("L2 normalization successful");
                n
            }
            Err(e) => {
                tracing::error!("L2 normalization failed: {:?}", e);
                return Err(e.context("L2 normalization failed"));
            }
        };

        // Convert to Vec<f32> - this transfers data from GPU to CPU
        // After this, we no longer need GPU access
        tracing::info!("Converting to Vec<f32>...");
        let embedding_vec = normalized
            .squeeze(0)
            .context("Failed to squeeze output")?
            .to_vec1::<f32>()
            .context("Failed to convert to Vec<f32>")?;

        // Explicitly drop the lock after all GPU operations are complete
        drop(inner);

        tracing::info!(
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
}
