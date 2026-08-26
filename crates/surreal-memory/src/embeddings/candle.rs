use super::{Embedding, EmbeddingPlanPart, EmbeddingService};
use anyhow::{Context, Result};
use async_trait::async_trait;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use hf_hub::{Repo, RepoType, api::tokio::ApiBuilder};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
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
    max_input_tokens: usize,
}

/// Thread-safe wrapper that serializes all GPU operations.
///
/// The inner state is held in `Arc<Mutex<_>>` so it can be moved into a
/// `spawn_blocking` task: the CPU-heavy tokenize + forward pass runs on a
/// blocking thread (acquiring the mutex there) and never starves the runtime.
pub struct CandleEmbeddings {
    inner: OnceCell<Arc<Mutex<CandleEmbeddingsInner>>>,
    model_id: String,
    model_revision: String,
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
            model_revision: std::env::var("LOCAL_EMBEDDING_MODEL_REVISION")
                .unwrap_or_else(|_| "main".to_string()),
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
    /// Download and model construction run outside the durable operation
    /// acceptance boundary. Production inference is supervised by the executor
    /// process; elapsed time never determines whether an operation succeeded.
    async fn ensure_loaded(&self) -> Result<&Arc<Mutex<CandleEmbeddingsInner>>> {
        self.inner
            .get_or_try_init(|| async {
                tracing::info!("Loading Candle embeddings model: {}", self.model_id);

                // Determine device (CUDA > Metal > CPU).
                //
                // `Device::new_metal`/`new_cuda` initialize the GPU stack through
                // synchronous FFI that can occupy the thread for tens of seconds
                // on a cold start. Called directly in this async block it blocked
                // a runtime worker, starved the executor's 250ms heartbeat, and
                // let the supervisor's watchdog declare the child nonresponsive
                // and SIGKILL it mid-initialization — 24 such restarts across 23
                // generations in production logs, each one immediately after the
                // "Metal available, using GPU" line. Offload it.
                let device = tokio::task::spawn_blocking(Self::get_device)
                    .await
                    .context("Embedding device selection task panicked")?
                    .context("Failed to get compute device")?;
                tracing::info!("Using device: {:?}", device);

                let (config_path, tokenizer_path, weights_path) =
                    Self::download_model(&self.model_id, &self.model_revision, &self.cache_dir)
                        .await
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
        let max_input_tokens = config.max_position_embeddings;
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
            max_input_tokens,
        })
    }

    fn get_device() -> Result<Device> {
        let device_preference = std::env::var("LOCAL_EMBEDDING_DEVICE").ok();
        if force_cpu(device_preference.as_deref())? {
            tracing::warn!("LOCAL_EMBEDDING_DEVICE=cpu: using the explicit degraded CPU backend");
            return Ok(Device::Cpu);
        }

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

    /// Resolve the directory hf-hub should treat as its cache root.
    ///
    /// `MODEL_CACHE_DIR` names the HuggingFace *home*; hf-hub stores repos in a
    /// `hub` subdirectory beneath it. Appending here keeps `MODEL_CACHE_DIR`
    /// meaning the same thing it means to every other HF tool, and keeps the
    /// Docker bind mount (`.../huggingface/hub`) valid.
    ///
    /// Idempotent: a path that already ends in `hub` is returned unchanged, so
    /// operators who point the variable straight at the hub directory (as the
    /// interim production fix did) are not sent to `.../hub/hub`.
    fn hub_cache_dir(cache_dir: &str) -> PathBuf {
        const HF_HUB_SUBDIR: &str = "hub";
        let path = PathBuf::from(cache_dir);
        if path.file_name().and_then(|name| name.to_str()) == Some(HF_HUB_SUBDIR) {
            path
        } else {
            path.join(HF_HUB_SUBDIR)
        }
    }

    async fn download_model(
        model_id: &str,
        model_revision: &str,
        cache_dir: &str,
    ) -> Result<(PathBuf, PathBuf, PathBuf)> {
        // hf-hub keeps repositories under `<hf-home>/hub`: both
        // `Cache::default()` (hf-hub-0.5.0 src/lib.rs:203-207) and
        // `Cache::from_env()` (src/lib.rs:42-49) append this component. But
        // `with_cache_dir` stores the path verbatim (`Cache::new`,
        // src/lib.rs:36-38), so the caller must append it. Passing
        // `MODEL_CACHE_DIR` unmodified pointed hf-hub one level *above* its own
        // data, orphaning an already-populated cache and re-downloading every
        // weight. It also broke credential lookup, since `token_path`
        // (src/lib.rs:59-64) derives the token file by popping this component.
        let hub_dir = Self::hub_cache_dir(cache_dir);

        tracing::info!(
            "Resolving model from Hugging Face: {}@{} (cache: {})",
            model_id,
            model_revision,
            hub_dir.display()
        );

        std::fs::create_dir_all(&hub_dir)
            .with_context(|| format!("create model cache directory {}", hub_dir.display()))?;

        let api = ApiBuilder::new()
            .with_cache_dir(hub_dir)
            .build()
            .context("build Hugging Face API client")?;
        let repo = api.repo(Repo::with_revision(
            model_id.to_string(),
            RepoType::Model,
            model_revision.to_string(),
        ));

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

        // hf-hub 0.5 builds its reqwest client with no connect or read timeout,
        // so a stalled transfer waits forever. Bound the whole download here:
        // without this the caller's watchdog is the only limit, and it kills the
        // process rather than returning a diagnosable error.
        let downloads = async {
            tokio::try_join!(
                async { config_fut.await.map_err(anyhow::Error::from) },
                async { tokenizer_fut.await.map_err(anyhow::Error::from) },
                weights_fut
            )
        };

        let (config_path, tokenizer_path, weights_path) =
            match tokio::time::timeout(Self::download_timeout(), downloads).await {
                Ok(result) => result?,
                Err(_) => anyhow::bail!(
                    "timed out after {:?} downloading model '{model_id}'; \
                     set MODEL_DOWNLOAD_TIMEOUT_SECS to allow longer, or pre-populate {cache_dir}",
                    Self::download_timeout()
                ),
            };

        tracing::info!("Model files downloaded successfully");

        Ok((config_path, tokenizer_path, weights_path))
    }

    /// Ceiling on a cold model download. Generous by default — a first pull of
    /// several hundred MB on a slow link is legitimate — but finite, so a
    /// stalled transfer fails with a clear error instead of hanging forever.
    fn download_timeout() -> Duration {
        const DEFAULT_DOWNLOAD_TIMEOUT_SECS: u64 = 600;
        Duration::from_secs(
            std::env::var("MODEL_DOWNLOAD_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|secs| *secs > 0)
                .unwrap_or(DEFAULT_DOWNLOAD_TIMEOUT_SECS),
        )
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
        validate_model_input_len(tokens.len(), inner.max_input_tokens)?;
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

fn force_cpu(value: Option<&str>) -> Result<bool> {
    match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(false),
        "cpu" => Ok(true),
        other => anyhow::bail!("LOCAL_EMBEDDING_DEVICE must be 'auto' or 'cpu', got '{other}'"),
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

    async fn plan(&self, text: &str) -> Result<Vec<EmbeddingPlanPart>> {
        let inner = Arc::clone(self.ensure_loaded().await?);
        let text = text.to_owned();
        tokio::task::spawn_blocking(move || {
            let inner = inner.blocking_lock();
            plan_token_windows(&inner.tokenizer, inner.max_input_tokens, &text)
        })
        .await
        .context("Embedding token planner task panicked")?
    }

    fn is_ready(&self) -> bool {
        self.is_loaded()
    }
}

fn validate_model_input_len(actual: usize, maximum: usize) -> Result<()> {
    if actual > maximum {
        anyhow::bail!(
            "input_too_long: tokenizer produced {actual} tokens for model capacity {maximum}"
        );
    }
    Ok(())
}

fn plan_token_windows(
    tokenizer: &Tokenizer,
    max_input_tokens: usize,
    text: &str,
) -> Result<Vec<EmbeddingPlanPart>> {
    let with_special = tokenizer
        .encode("", true)
        .map_err(|error| anyhow::anyhow!("Tokenization failed: {error}"))?;
    let special_tokens = with_special.len();
    let usable = max_input_tokens
        .checked_sub(special_tokens)
        .filter(|value| *value > 0)
        .context("embedding model capacity does not leave room for content tokens")?;
    let source = tokenizer
        .encode(text, false)
        .map_err(|error| anyhow::anyhow!("Tokenization failed: {error}"))?;
    let ids = source.get_ids();
    if ids.len() + special_tokens <= max_input_tokens {
        return Ok(vec![plan_part(0, 0, ids.len(), ids, text.to_owned())]);
    }

    // The overlap is a fixed token-domain constant, not a timing heuristic.
    // It preserves boundary context while every encoded part is proven below
    // to fit the active model's exact capacity.
    let overlap = 32usize.min(usable.saturating_sub(1));
    let step = usable - overlap;
    let mut parts = Vec::new();
    let mut start = 0usize;
    while start < ids.len() {
        let mut end = (start + usable).min(ids.len());
        let mut content = tokenizer
            .decode(&ids[start..end], true)
            .map_err(|error| anyhow::anyhow!("Token decode failed: {error}"))?;

        // Decoding and encoding can normalize whitespace differently for some
        // tokenizers. Shrink deterministically until the actual model input,
        // including special tokens, is within capacity.
        loop {
            let verified = tokenizer
                .encode(content.as_str(), true)
                .map_err(|error| anyhow::anyhow!("Token verification failed: {error}"))?;
            if verified.len() <= max_input_tokens {
                break;
            }
            end = end
                .checked_sub(1)
                .filter(|candidate| *candidate > start)
                .context("unable to construct a model-safe token window")?;
            content = tokenizer
                .decode(&ids[start..end], true)
                .map_err(|error| anyhow::anyhow!("Token decode failed: {error}"))?;
        }

        parts.push(plan_part(
            parts.len(),
            start,
            end,
            &ids[start..end],
            content,
        ));
        if end == ids.len() {
            break;
        }
        start = start.saturating_add(step).min(end);
    }
    Ok(parts)
}

fn plan_part(
    part_index: usize,
    token_start: usize,
    token_end: usize,
    ids: &[u32],
    content: String,
) -> EmbeddingPlanPart {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(ids));
    for id in ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    EmbeddingPlanPart {
        part_index,
        token_start,
        token_end,
        token_count: token_end.saturating_sub(token_start),
        token_hash: Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenizers::{
        models::wordlevel::WordLevel, pre_tokenizers::whitespace::Whitespace,
        processors::template::TemplateProcessing,
    };

    fn boundary_tokenizer() -> Tokenizer {
        let vocab = [
            ("[UNK]".to_owned(), 0),
            ("[CLS]".to_owned(), 1),
            ("[SEP]".to_owned(), 2),
            ("word".to_owned(), 3),
        ]
        .into_iter()
        .collect();
        let model = WordLevel::builder()
            .vocab(vocab)
            .unk_token("[UNK]".to_owned())
            .build()
            .unwrap();
        let mut tokenizer = Tokenizer::new(model);
        tokenizer.with_pre_tokenizer(Some(Whitespace));
        tokenizer.with_post_processor(Some(
            TemplateProcessing::builder()
                .try_single("[CLS] $A [SEP]")
                .unwrap()
                .special_tokens(vec![("[CLS]", 1), ("[SEP]", 2)])
                .build()
                .unwrap(),
        ));
        tokenizer
    }

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

    #[test]
    fn device_preference_is_explicit_and_fail_closed() {
        assert!(!force_cpu(None).unwrap());
        assert!(!force_cpu(Some("auto")).unwrap());
        assert!(force_cpu(Some("CPU")).unwrap());
        assert!(force_cpu(Some("metal")).is_err());
    }

    #[test]
    fn model_boundary_guard_accepts_below_and_at_capacity_only() {
        assert!(validate_model_input_len(510, 512).is_ok());
        assert!(validate_model_input_len(512, 512).is_ok());
        assert!(validate_model_input_len(513, 512).is_err());
    }

    #[test]
    fn planner_uses_exact_special_token_capacity_and_stable_token_windows() {
        let tokenizer = boundary_tokenizer();
        let below = plan_token_windows(&tokenizer, 6, "word word word").unwrap();
        let at = plan_token_windows(&tokenizer, 6, "word word word word").unwrap();
        let above = plan_token_windows(&tokenizer, 6, "word word word word word").unwrap();

        assert_eq!(below.len(), 1);
        assert_eq!(below[0].token_count, 3);
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].token_count, 4);
        assert!(above.len() > 1);
        assert_eq!(above.first().unwrap().token_start, 0);
        assert_eq!(above.last().unwrap().token_end, 5);
        assert!(
            above
                .windows(2)
                .all(|pair| pair[1].token_start <= pair[0].token_end)
        );
        assert!(above.iter().all(|part| {
            tokenizer
                .encode(part.content.as_str(), true)
                .map(|encoding| encoding.len() <= 6)
                .unwrap_or(false)
        }));
        assert_eq!(
            above,
            plan_token_windows(&tokenizer, 6, "word word word word word").unwrap()
        );
    }

    /// Certification test for the authored tokenizer and model config. It is
    /// ignored in the hermetic unit suite because the model files are installed
    /// artifacts, then run explicitly on the target host before activation.
    #[test]
    #[ignore = "requires SURREAL_REAL_TOKENIZER and SURREAL_REAL_MODEL_CONFIG"]
    fn real_tokenizer_proves_below_at_and_above_model_capacity() {
        let tokenizer_path = std::env::var("SURREAL_REAL_TOKENIZER")
            .expect("SURREAL_REAL_TOKENIZER must name the installed tokenizer.json");
        let config_path = std::env::var("SURREAL_REAL_MODEL_CONFIG")
            .expect("SURREAL_REAL_MODEL_CONFIG must name the installed config.json");
        let tokenizer = Tokenizer::from_file(tokenizer_path).expect("load authored tokenizer");
        let config: serde_json::Value = serde_json::from_reader(
            std::fs::File::open(config_path).expect("open authored model config"),
        )
        .expect("parse authored model config");
        let maximum = config["max_position_embeddings"]
            .as_u64()
            .expect("model config max_position_embeddings") as usize;
        let special_tokens = tokenizer.encode("", true).unwrap().len();
        let usable = maximum - special_tokens;

        fn repeated_word_tokens(tokenizer: &Tokenizer, count: usize) -> String {
            let text = std::iter::repeat_n("word", count)
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(tokenizer.encode(text.as_str(), false).unwrap().len(), count);
            text
        }

        let below = repeated_word_tokens(&tokenizer, usable - 1);
        let at = repeated_word_tokens(&tokenizer, usable);
        let above = repeated_word_tokens(&tokenizer, usable + 1);
        let below_plan = plan_token_windows(&tokenizer, maximum, &below).unwrap();
        let at_plan = plan_token_windows(&tokenizer, maximum, &at).unwrap();
        let above_plan = plan_token_windows(&tokenizer, maximum, &above).unwrap();

        assert_eq!(below_plan.len(), 1);
        assert_eq!(below_plan[0].token_count, usable - 1);
        assert_eq!(at_plan.len(), 1);
        assert_eq!(at_plan[0].token_count, usable);
        assert!(above_plan.len() > 1);
        assert_eq!(above_plan.first().unwrap().token_start, 0);
        assert_eq!(above_plan.last().unwrap().token_end, usable + 1);
        assert!(above_plan.iter().all(|part| {
            tokenizer
                .encode(part.content.as_str(), true)
                .map(|encoded| encoded.len() <= maximum)
                .unwrap_or(false)
        }));
    }

    #[test]
    fn hub_cache_dir_appends_the_hf_hub_subdirectory() {
        // MODEL_CACHE_DIR names the HF home; hf-hub stores repos under <home>/hub.
        // Passing the home verbatim orphaned a populated cache in production and
        // triggered a full re-download of the weights.
        assert_eq!(
            CandleEmbeddings::hub_cache_dir("/Users/someone/.cache/huggingface"),
            std::path::PathBuf::from("/Users/someone/.cache/huggingface/hub")
        );
    }

    #[test]
    fn hub_cache_dir_is_idempotent_when_already_pointed_at_hub() {
        // Operators (and the interim production hotfix) may point the variable
        // straight at the hub directory. That must not resolve to `.../hub/hub`.
        assert_eq!(
            CandleEmbeddings::hub_cache_dir("/Users/someone/.cache/huggingface/hub"),
            std::path::PathBuf::from("/Users/someone/.cache/huggingface/hub")
        );
    }

    #[test]
    fn hub_cache_dir_handles_a_relative_configured_path() {
        // `.env.example` ships MODEL_CACHE_DIR=./models.
        assert_eq!(
            CandleEmbeddings::hub_cache_dir("./models"),
            std::path::PathBuf::from("./models/hub")
        );
    }
}
