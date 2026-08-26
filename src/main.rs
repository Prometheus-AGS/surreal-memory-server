use anyhow::{Context, Result};
use std::sync::Arc;
use surreal_memory::storage::migrations::{inspect_legacy_enum_data, repair_legacy_enum_data};
use surreal_memory_server::{
    api,
    config::Config,
    embeddings::{self, EmbeddingService, create_embedding_service},
    executor::{SupervisedEmbeddingService, run_embedding_executor},
    mcp::MemoryMcpServer,
    storage::{MemoryStorage, create_storage},
    workers,
};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::opt::auth::Root;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Serve,
    RepairData { apply: bool },
    EmbeddingExecutor,
    Version,
}

/// Parse retry configuration from environment variables with sensible defaults.
fn parse_retry_config_from_env() -> surreal_memory::RetryConfig {
    use std::env;

    let max_connect_retries = env::var("SURREAL_MAX_CONNECT_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    let max_operation_retries = env::var("SURREAL_MAX_OPERATION_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let base_retry_delay_ms = env::var("SURREAL_BASE_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);

    let max_retry_delay_ms = env::var("SURREAL_MAX_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000);

    let jitter_factor = env::var("SURREAL_RETRY_JITTER_FACTOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.25);

    let operation_deadline_ms = env::var("SURREAL_OPERATION_DEADLINE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000);

    let query_timeout_ms = env::var("SURREAL_QUERY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);

    surreal_memory::RetryConfig {
        max_connect_retries,
        max_operation_retries,
        base_retry_delay_ms,
        max_retry_delay_ms,
        jitter_factor,
        operation_deadline_ms,
        query_timeout_ms,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = parse_command(std::env::args().skip(1))?;
    if command == Command::Version {
        println!("surreal-memory-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    init_logging();
    install_panic_hook();

    match command {
        Command::Serve => {}
        Command::RepairData { apply } => {
            return run_repair_command(apply).await;
        }
        Command::EmbeddingExecutor => {
            return run_embedding_executor().await;
        }
        Command::Version => unreachable!("version exits before runtime initialization"),
    }

    tracing::info!(
        "🚀 Starting Rust Memory MCP Server v{}",
        env!("CARGO_PKG_VERSION")
    );

    let config = load_config().await?;
    let embedding_service = init_embedding_service(&config).await?;
    let retry_config = parse_retry_config_from_env();

    if config.embedding_warmup {
        warmup_embedding(Arc::clone(&embedding_service)).await;
    }

    let api_port: u16 = std::env::var("API_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);

    // The API layer keeps a handle to the embedding service so `/health` can
    // report true readiness via `EmbeddingService::is_ready()`.
    let health_embedding = Arc::clone(&embedding_service);
    let storage = init_storage(config, embedding_service, retry_config).await?;

    // Graceful shutdown channel — send `true` to stop all workers.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn the signal handler that broadcasts the shutdown signal.
    //
    // SIGTERM matters as much as SIGINT here: launchd, systemd, Docker and
    // Kubernetes all stop a service with SIGTERM. Handling only Ctrl+C meant
    // every managed stop killed the process with no log line at all, which is
    // why a terminated server left no diagnostic behind.
    let shutdown_tx2 = shutdown_tx.clone();
    tokio::spawn(async move {
        let reason = wait_for_shutdown_signal().await;
        tracing::info!(signal = reason, "🛑 Received shutdown signal");
        let _ = shutdown_tx2.send(true);
    });

    // ── Background TTL worker ─────────────────────────────────────────────────
    let ttl_storage = Arc::clone(&storage);
    let ttl_rx = shutdown_rx.clone();
    let ttl_handle =
        tokio::spawn(async move { workers::ttl::run_ttl_worker(ttl_storage, ttl_rx).await });

    // ── Axum REST API + HTTP/SSE MCP ─────────────────────────────────────────
    let api_storage = Arc::clone(&storage);
    let api_handle =
        tokio::spawn(async move { run_api_server(api_storage, api_port, health_embedding).await });

    // ── MCP stdio ─────────────────────────────────────────────────────────────
    let enable_stdio_mcp = std::env::var("MCP_STDIO")
        .unwrap_or_else(|_| "true".to_string())
        .to_lowercase()
        != "false";

    let mcp_handle = tokio::spawn(async move {
        if enable_stdio_mcp {
            run_mcp_server(storage).await
        } else {
            // Keep the future pending forever so tokio::select! doesn't exit
            std::future::pending().await
        }
    });

    // Whichever task ends first ends the process. Record whether it ended
    // cleanly so the final line reports what actually happened.
    let clean_exit = tokio::select! {
        result = api_handle => {
            match result {
                Ok(Ok(())) => { tracing::info!("REST API server stopped"); true }
                Ok(Err(e)) => { tracing::error!("REST API server error: {}", e); false }
                Err(e) => { tracing::error!("REST API task panic: {}", e); false }
            }
        }
        result = mcp_handle => {
            if enable_stdio_mcp {
                match result {
                    Ok(Ok(())) => { tracing::info!("MCP server stopped"); true }
                    Ok(Err(e)) => { tracing::error!("MCP server error: {}", e); false }
                    Err(e) => { tracing::error!("MCP task panic: {}", e); false }
                }
            } else {
                true
            }
        }
        result = ttl_handle => {
            match result {
                Ok(Ok(())) => { tracing::info!("TTL worker stopped"); true }
                Ok(Err(e)) => { tracing::error!("TTL worker error: {}", e); false }
                Err(e) => { tracing::error!("TTL worker panic: {}", e); false }
            }
        }
    };

    // Broadcast shutdown to any remaining tasks.
    let _ = shutdown_tx.send(true);
    if clean_exit {
        tracing::info!("👋 Server shut down gracefully");
        Ok(())
    } else {
        // Previously this logged "shut down gracefully" even after a task
        // error or panic, and returned Ok — so a crash was indistinguishable
        // from a clean stop in both the log and the exit code.
        tracing::error!("💥 Server exiting after a task failure");
        anyhow::bail!("server exited because a supervised task failed")
    }
}

/// Route panics through `tracing` before the default handler runs.
///
/// A panic inside a detached `tokio::spawn` — an SSE session task, the executor
/// supervisor — is otherwise swallowed: the task dies, the message goes to
/// stderr unstructured or not at all, and the surviving process leaves no
/// record of what failed. This makes every panic a structured ERROR carrying
/// the payload, location, and thread name.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_owned());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_owned());
        let thread = std::thread::current();
        tracing::error!(
            panic.payload = %payload,
            panic.location = %location,
            panic.thread = thread.name().unwrap_or("<unnamed>"),
            "thread panicked"
        );
        previous(info);
    }));
}

/// Resolve when the process is asked to stop, reporting which signal arrived.
///
/// SIGTERM is the signal every supervisor actually sends (launchd, systemd,
/// Docker, Kubernetes); SIGINT only arrives from an interactive Ctrl+C.
async fn wait_for_shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                tracing::error!(%error, "failed to install SIGTERM handler");
                let _ = tokio::signal::ctrl_c().await;
                return "SIGINT";
            }
        };
        tokio::select! {
            _ = sigterm.recv() => "SIGTERM",
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::error!(%error, "failed to listen for SIGINT");
                }
                "SIGINT"
            }
        }
    }

    #[cfg(not(unix))]
    {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to listen for shutdown signal");
        }
        "SIGINT"
    }
}

fn init_logging() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "surreal_memory_server=info,surreal=warn".into()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(false)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false),
        )
        .init();
}

fn parse_command<I, S>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(Command::Serve);
    }

    match args[0].as_str() {
        "--version" | "-V" if args.len() == 1 => Ok(Command::Version),
        "--version" | "-V" => {
            anyhow::bail!("Usage: surreal-memory-server [--version|-V]")
        }
        "repair-data" => {
            let apply = args.iter().skip(1).any(|arg| arg == "--apply");
            let dry_run = args.iter().skip(1).any(|arg| arg == "--dry-run");

            if apply == dry_run {
                anyhow::bail!("Usage: surreal-memory-server repair-data [--dry-run|--apply]");
            }

            Ok(Command::RepairData { apply })
        }
        "embedding-executor" => Ok(Command::EmbeddingExecutor),
        other => anyhow::bail!("Unknown command '{}'", other),
    }
}

fn load_surreal_config_without_embeddings() -> Result<surreal_memory::SurrealConfig> {
    dotenvy::dotenv().ok();

    let mode = match std::env::var("SURREAL_MODE")
        .unwrap_or_else(|_| "embedded".to_string())
        .to_lowercase()
        .as_str()
    {
        "server" => surreal_memory::storage::surreal::SurrealMode::Server,
        _ => surreal_memory::storage::surreal::SurrealMode::Embedded,
    };

    Ok(surreal_memory::SurrealConfig {
        mode,
        endpoint: std::env::var("SURREAL_ENDPOINT").ok(),
        embedded_path: std::env::var("SURREAL_PATH")
            .ok()
            .or_else(|| Some("./data/memory.db".to_string())),
        username: std::env::var("SURREAL_USERNAME").ok(),
        password: std::env::var("SURREAL_PASSWORD").ok(),
        namespace: std::env::var("SURREAL_NAMESPACE").unwrap_or_else(|_| "memory".to_string()),
        database: std::env::var("SURREAL_DATABASE").unwrap_or_else(|_| "mcp".to_string()),
        retry: parse_retry_config_from_env(),
    })
}

async fn connect_surreal_for_repair(
    config: &surreal_memory::SurrealConfig,
) -> Result<Surreal<Any>> {
    let db = match &config.mode {
        surreal_memory::storage::surreal::SurrealMode::Embedded => {
            let path = config
                .embedded_path
                .as_ref()
                .context("Embedded path required for embedded repair mode")?;
            surrealdb::engine::any::connect(format!("rocksdb://{}", path)).await?
        }
        surreal_memory::storage::surreal::SurrealMode::Server => {
            let endpoint = config
                .endpoint
                .as_ref()
                .context("SURREAL_ENDPOINT is required for server repair mode")?;
            surrealdb::engine::any::connect(endpoint).await?
        }
    };

    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        db.signin(Root {
            username: username.clone(),
            password: password.clone(),
        })
        .await
        .context("Failed to sign in to SurrealDB")?;
    }

    db.use_ns(&config.namespace)
        .use_db(&config.database)
        .await
        .context("Failed to select namespace/database for repair")?;

    Ok(db)
}

async fn run_repair_command(apply: bool) -> Result<()> {
    let config = load_surreal_config_without_embeddings()?;
    let db = connect_surreal_for_repair(&config).await?;

    if apply {
        let report = repair_legacy_enum_data(&db).await?;
        print_repair_report("apply", &report);
    } else {
        let report = inspect_legacy_enum_data(&db).await?;
        print_repair_report("dry-run", &report);
        if report.has_issues() {
            anyhow::bail!(
                "repair-data --dry-run found {} unrecognized enum value(s)",
                report.issues.len()
            );
        }
    }

    Ok(())
}

fn print_repair_report(mode: &str, report: &surreal_memory::storage::migrations::RepairReport) {
    println!(
        "repair-data {}: scanned_records={} planned_repairs={} repairs_applied={} issues={}",
        mode,
        report.scanned_records,
        report.planned_repairs(),
        report.repairs_applied,
        report.issues.len()
    );

    for change in &report.changes {
        println!(
            "repair {}.{} {} => {}",
            change.table, change.field, change.record_id, change.normalized_value
        );
    }

    for issue in &report.issues {
        println!(
            "issue {}.{} {} raw={} error={}",
            issue.table, issue.field, issue.record_id, issue.raw_value, issue.message
        );
    }
}

async fn load_config() -> Result<Config> {
    tracing::info!("📋 Loading configuration...");
    let config = Config::from_env().context("Failed to load configuration")?;
    tracing::info!(
        "✅ Configuration loaded — SurrealDB mode: {:?}",
        config.surreal_mode
    );
    match &config.embedding_provider {
        embeddings::EmbeddingProvider::OpenAI { model, .. } => {
            tracing::info!("   Embedding: OpenAI ({})", model);
        }
        embeddings::EmbeddingProvider::Cohere { model, .. } => {
            tracing::info!("   Embedding: Cohere ({})", model);
        }
        embeddings::EmbeddingProvider::Local { model_id, .. } => {
            tracing::info!("   Embedding: Local ({})", model_id);
        }
        #[cfg(feature = "palace")]
        embeddings::EmbeddingProvider::Fast => {
            tracing::info!("   Embedding: FastEmbed (palace)");
        }
    }
    Ok(config)
}

async fn init_embedding_service(config: &Config) -> Result<Arc<dyn EmbeddingService>> {
    tracing::info!("🧠 Initializing embedding service...");
    let configured = create_embedding_service(config.embedding_provider.clone())
        .await
        .context("Failed to create embedding service")?;
    let dimensions = configured.dimensions();
    drop(configured);
    let watchdog_ms = std::env::var("SURREAL_EXECUTOR_WATCHDOG_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30_000);
    let service = SupervisedEmbeddingService::new(
        std::env::current_exe().context("resolve server executable for embedding supervisor")?,
        dimensions,
        std::time::Duration::from_millis(watchdog_ms),
    );
    tracing::info!(
        "🧠 Embedding service configured ({} dimensions); model loads on warmup or first use",
        service.dimensions()
    );
    Ok(Arc::new(service))
}

async fn init_storage(
    config: Config,
    embedding_service: Arc<dyn EmbeddingService>,
    retry_config: surreal_memory::RetryConfig,
) -> Result<Arc<dyn MemoryStorage>> {
    tracing::info!("💾 Initializing storage...");
    let storage = create_storage(&config, embedding_service, retry_config)
        .await
        .context("Failed to initialize SurrealDB storage")?;
    tracing::info!("✅ Storage initialized successfully");
    Ok(storage)
}

async fn run_mcp_server(storage: Arc<dyn MemoryStorage>) -> Result<()> {
    tracing::info!("🎯 Starting MCP server on stdio transport...");
    let server = MemoryMcpServer::new(storage);
    server.run().await.context("MCP server error")
}

/// Eagerly loads the embedding model so the first user-facing write does not
/// pay the cold-load latency. A warmup failure is logged but not fatal — the
/// model will be retried lazily on first use. Readiness is reported via
/// `EmbeddingService::is_ready()`, so `/health` stays accurate regardless of
/// whether warmup ran or succeeded.
async fn warmup_embedding(service: Arc<dyn EmbeddingService>) {
    tracing::info!("🔥 Warming up embedding model...");
    match service.embed("warmup").await {
        Ok(_) => {
            tracing::info!("✅ Embedding model warmed up and ready");
        }
        Err(e) => {
            tracing::warn!(error = %e, "Embedding warmup failed; model will load lazily on first use");
        }
    }
}

async fn run_api_server(
    storage: Arc<dyn MemoryStorage>,
    port: u16,
    embedding_service: Arc<dyn EmbeddingService>,
) -> Result<()> {
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("🌐 Starting REST API + HTTP MCP server on http://{}", addr);
    let router = api::build_router(storage, embedding_service);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind REST API port")?;
    axum::serve(listener, router)
        .await
        .context("REST API server error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Global lock to ensure tests don't run concurrently and mutate env vars
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_retry_config_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Clear any environment variables
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 10);
        assert_eq!(config.max_operation_retries, 3);
        assert_eq!(config.base_retry_delay_ms, 100);
        assert_eq!(config.max_retry_delay_ms, 5000);
        assert_eq!(config.jitter_factor, 0.25);
    }

    #[test]
    fn test_retry_config_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();

        unsafe {
            std::env::set_var("SURREAL_MAX_CONNECT_RETRIES", "20");
            std::env::set_var("SURREAL_MAX_OPERATION_RETRIES", "5");
            std::env::set_var("SURREAL_BASE_RETRY_DELAY_MS", "200");
            std::env::set_var("SURREAL_MAX_RETRY_DELAY_MS", "10000");
            std::env::set_var("SURREAL_RETRY_JITTER_FACTOR", "0.5");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 20);
        assert_eq!(config.max_operation_retries, 5);
        assert_eq!(config.base_retry_delay_ms, 200);
        assert_eq!(config.max_retry_delay_ms, 10000);
        assert_eq!(config.jitter_factor, 0.5);

        // Cleanup
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }
    }

    #[test]
    fn test_retry_config_partial_env() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Set only some variables, others should use defaults
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::set_var("SURREAL_BASE_RETRY_DELAY_MS", "500");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 10); // default
        assert_eq!(config.max_operation_retries, 3); // default
        assert_eq!(config.base_retry_delay_ms, 500); // custom
        assert_eq!(config.max_retry_delay_ms, 5000); // default
        assert_eq!(config.jitter_factor, 0.25); // default

        // Cleanup
        unsafe {
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
        }
    }

    #[test]
    fn test_retry_config_invalid_values() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Clear all first
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_MAX_OPERATION_RETRIES");
            std::env::remove_var("SURREAL_BASE_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_MAX_RETRY_DELAY_MS");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }

        // Invalid values should fall back to defaults
        unsafe {
            std::env::set_var("SURREAL_MAX_CONNECT_RETRIES", "not_a_number");
            std::env::set_var("SURREAL_RETRY_JITTER_FACTOR", "invalid");
        }

        let config = parse_retry_config_from_env();

        assert_eq!(config.max_connect_retries, 10); // default due to parse error
        assert_eq!(config.jitter_factor, 0.25); // default due to parse error

        // Cleanup
        unsafe {
            std::env::remove_var("SURREAL_MAX_CONNECT_RETRIES");
            std::env::remove_var("SURREAL_RETRY_JITTER_FACTOR");
        }
    }

    #[test]
    fn test_parse_command_defaults_to_serve() {
        assert_eq!(parse_command(Vec::<String>::new()).unwrap(), Command::Serve);
    }

    #[test]
    fn test_parse_command_accepts_repair_apply() {
        assert_eq!(
            parse_command(vec!["repair-data".to_string(), "--apply".to_string()]).unwrap(),
            Command::RepairData { apply: true }
        );
    }

    #[test]
    fn test_parse_command_accepts_embedding_executor() {
        assert_eq!(
            parse_command(vec!["embedding-executor".to_string()]).unwrap(),
            Command::EmbeddingExecutor
        );
    }

    #[test]
    fn test_parse_command_requires_explicit_repair_mode() {
        assert!(parse_command(vec!["repair-data".to_string()]).is_err());
        assert!(
            parse_command(vec![
                "repair-data".to_string(),
                "--dry-run".to_string(),
                "--apply".to_string()
            ])
            .is_err()
        );
    }
}
