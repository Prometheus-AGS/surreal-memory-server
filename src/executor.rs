use std::sync::Mutex as StdMutex;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use surreal_memory::embeddings::{
    Embedding, EmbeddingPlanPart, EmbeddingService, ExecutorEvent, ExecutorEventKind,
    ExecutorSnapshot,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, broadcast},
};

#[derive(Debug, Serialize, Deserialize)]
struct ExecutorRequest {
    request_id: u64,
    operation_id: Option<String>,
    command: ExecutorCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum ExecutorCommand {
    Plan { text: String },
    Embed { part_index: usize, text: String },
    EmbedBatch { texts: Vec<String> },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
enum ExecutorMessage {
    Progress {
        request_id: u64,
        phase: String,
    },
    Completed {
        request_id: u64,
        result: ExecutorResult,
    },
    Failed {
        request_id: u64,
        error: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum ExecutorResult {
    Plan { parts: Vec<EmbeddingPlanPart> },
    Embedding { embedding: Embedding },
    Batch { embeddings: Vec<Embedding> },
}

struct ChildState {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    generation: u64,
}

/// Internal classification for request failures. Retriable failures are
/// transport or protocol desynchronizations where the supervisor has already
/// restarted the executor child, so issuing the request again is safe —
/// embedding is a pure function with no side effects. Non-retriable failures
/// are executor-reported errors or supervisor invariants that a retry cannot
/// fix.
struct RequestFailure {
    retriable: bool,
    error: anyhow::Error,
}

impl RequestFailure {
    fn retriable(error: anyhow::Error) -> Self {
        Self {
            retriable: true,
            error,
        }
    }

    fn fatal(error: anyhow::Error) -> Self {
        Self {
            retriable: false,
            error,
        }
    }
}

#[derive(Clone)]
struct OperationBaseline {
    persisted_exit_count: u64,
    session_exit_start: u64,
    last_exit: Option<String>,
    error: Option<String>,
}

/// Long-lived embedding subprocess supervised by progress messages. The
/// watchdog only decides whether the child is responsive; operation success is
/// still determined exclusively by durable ledger and part states.
pub struct SupervisedEmbeddingService {
    executable: PathBuf,
    dimensions: usize,
    watchdog: Duration,
    state: Mutex<Option<ChildState>>,
    next_request_id: AtomicU64,
    next_generation: AtomicU64,
    progress_seq: AtomicU64,
    current_generation: AtomicU64,
    exit_count: AtomicU64,
    last_exit: StdMutex<Option<String>>,
    last_error: StdMutex<Option<String>>,
    operation_baselines: StdMutex<HashMap<String, OperationBaseline>>,
    ready: AtomicBool,
    events_tx: broadcast::Sender<ExecutorEvent>,
    child_env: Vec<(String, String)>,
}

impl SupervisedEmbeddingService {
    pub fn new(executable: PathBuf, dimensions: usize, watchdog: Duration) -> Self {
        Self::with_child_env(executable, dimensions, watchdog, Vec::new())
    }

    pub fn with_child_env(
        executable: PathBuf,
        dimensions: usize,
        watchdog: Duration,
        child_env: Vec<(String, String)>,
    ) -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            executable,
            dimensions,
            watchdog,
            state: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            next_generation: AtomicU64::new(1),
            progress_seq: AtomicU64::new(0),
            current_generation: AtomicU64::new(0),
            exit_count: AtomicU64::new(0),
            last_exit: StdMutex::new(None),
            last_error: StdMutex::new(None),
            operation_baselines: StdMutex::new(HashMap::new()),
            ready: AtomicBool::new(false),
            events_tx,
            child_env,
        }
    }

    fn emit(
        &self,
        operation_id: Option<&str>,
        generation: u64,
        kind: ExecutorEventKind,
        message: Option<String>,
    ) {
        let progress_seq = self.progress_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.current_generation.store(generation, Ordering::SeqCst);
        if matches!(
            kind,
            ExecutorEventKind::Exited | ExecutorEventKind::Nonresponsive
        ) {
            self.exit_count.fetch_add(1, Ordering::SeqCst);
            *self.last_exit.lock().expect("executor exit mutex") = message.clone();
            if let Some(operation_id) = operation_id
                && let Some(baseline) = self
                    .operation_baselines
                    .lock()
                    .expect("operation baseline mutex")
                    .get_mut(operation_id)
            {
                baseline.last_exit = message.clone();
            }
        }
        if matches!(
            kind,
            ExecutorEventKind::Exited | ExecutorEventKind::Nonresponsive | ExecutorEventKind::Error
        ) {
            *self.last_error.lock().expect("executor error mutex") = message.clone();
            if let Some(operation_id) = operation_id
                && let Some(baseline) = self
                    .operation_baselines
                    .lock()
                    .expect("operation baseline mutex")
                    .get_mut(operation_id)
            {
                baseline.error = message.clone();
            }
        }
        let _ = self.events_tx.send(ExecutorEvent {
            operation_id: operation_id.map(str::to_owned),
            generation,
            progress_seq,
            kind,
            message,
        });
    }

    async fn spawn_child(&self) -> Result<ChildState> {
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
        let mut command = Command::new(&self.executable);
        command
            .arg("embedding-executor")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for (name, value) in &self.child_env {
            command.env(name, value);
        }
        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to start embedding executor '{}'",
                self.executable.display()
            )
        })?;
        let stdin = child.stdin.take().context("executor stdin unavailable")?;
        let stdout = child.stdout.take().context("executor stdout unavailable")?;
        self.emit(None, generation, ExecutorEventKind::Started, None);
        Ok(ChildState {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            generation,
        })
    }

    async fn stop_child(
        &self,
        child: &mut ChildState,
        operation_id: Option<&str>,
        kind: ExecutorEventKind,
        message: String,
    ) -> Result<()> {
        self.ready.store(false, Ordering::SeqCst);
        let status = match child
            .child
            .try_wait()
            .context("failed to inspect embedding executor")?
        {
            Some(status) => status,
            None => {
                child
                    .child
                    .kill()
                    .await
                    .context("failed to terminate embedding executor")?;
                child
                    .child
                    .wait()
                    .await
                    .context("failed to reap embedding executor")?
            }
        };
        self.emit(
            operation_id,
            child.generation,
            kind,
            Some(format!("{message}; status={status}")),
        );
        Ok(())
    }

    async fn request(
        &self,
        operation_id: Option<&str>,
        command: ExecutorCommand,
    ) -> Result<ExecutorResult> {
        // Interactive callers (no durable operation) have no retry layer
        // above them, so retry once after a transport or protocol
        // desynchronization; request_once has already restarted the executor
        // child at that point, and embedding is side-effect free.
        // Operation-scoped callers are instead resumed by the operations
        // layer through the durable ledger, so they must observe the failure.
        let retry_on_desync = operation_id.is_none();
        match self.request_once(operation_id, &command).await {
            Ok(result) => Ok(result),
            Err(failure) if failure.retriable && retry_on_desync => {
                self.emit(
                    operation_id,
                    self.current_generation.load(Ordering::SeqCst),
                    ExecutorEventKind::Progress,
                    Some(format!(
                        "retrying on a fresh executor after: {}",
                        failure.error
                    )),
                );
                self.request_once(operation_id, &command)
                    .await
                    .map_err(|failure| failure.error)
            }
            Err(failure) => Err(failure.error),
        }
    }

    async fn request_once(
        &self,
        operation_id: Option<&str>,
        command: &ExecutorCommand,
    ) -> std::result::Result<ExecutorResult, RequestFailure> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        let request = ExecutorRequest {
            request_id,
            operation_id: operation_id.map(str::to_owned),
            command: command.clone(),
        };
        let encoded = serde_json::to_vec(&request)
            .context("serialize executor request")
            .map_err(RequestFailure::fatal)?;
        let mut state = self.state.lock().await;
        if state.is_none() {
            *state = Some(self.spawn_child().await.map_err(RequestFailure::fatal)?);
        }
        let child = state.as_mut().expect("executor child initialized");
        let generation = child.generation;
        if let Err(error) = async {
            child.stdin.write_all(&encoded).await?;
            child.stdin.write_all(b"\n").await?;
            child.stdin.flush().await
        }
        .await
        {
            self.restart_child(
                &mut state,
                operation_id,
                ExecutorEventKind::Exited,
                format!("executor request write failed: {error}"),
            )
            .await?;
            return Err(RequestFailure::retriable(anyhow::anyhow!(
                "embedding executor generation {generation} exited before request write"
            )));
        }

        loop {
            let mut line = String::new();
            let read = tokio::time::timeout(self.watchdog, child.stdout.read_line(&mut line)).await;
            match read {
                Err(_) => {
                    self.restart_child(
                        &mut state,
                        operation_id,
                        ExecutorEventKind::Nonresponsive,
                        "executor progress watchdog observed no progress".to_owned(),
                    )
                    .await?;
                    return Err(RequestFailure::retriable(anyhow::anyhow!(
                        "embedding executor generation {generation} was nonresponsive and restarted"
                    )));
                }
                Ok(Err(error)) => {
                    self.restart_child(
                        &mut state,
                        operation_id,
                        ExecutorEventKind::Exited,
                        format!("executor response read failed: {error}"),
                    )
                    .await?;
                    return Err(RequestFailure::retriable(anyhow::anyhow!(
                        "embedding executor generation {generation} exited"
                    )));
                }
                Ok(Ok(0)) => {
                    self.restart_child(
                        &mut state,
                        operation_id,
                        ExecutorEventKind::Exited,
                        "executor response stream closed".to_owned(),
                    )
                    .await?;
                    return Err(RequestFailure::retriable(anyhow::anyhow!(
                        "embedding executor generation {generation} exited"
                    )));
                }
                Ok(Ok(_)) => {}
            }

            let message: ExecutorMessage = match serde_json::from_str(line.trim_end()) {
                Ok(message) => message,
                Err(error) => {
                    // An undecodable line means the child is no longer
                    // speaking the protocol; do not trust the stream again.
                    self.restart_child(
                        &mut state,
                        operation_id,
                        ExecutorEventKind::Exited,
                        format!("decode executor response failed: {error}"),
                    )
                    .await?;
                    return Err(RequestFailure::retriable(anyhow::anyhow!(
                        "embedding executor generation {generation} sent an undecodable response and restarted"
                    )));
                }
            };
            match message {
                ExecutorMessage::Progress {
                    request_id: response_id,
                    phase,
                } if response_id == request_id => self.emit(
                    operation_id,
                    generation,
                    ExecutorEventKind::Progress,
                    Some(phase),
                ),
                ExecutorMessage::Completed {
                    request_id: response_id,
                    result,
                } if response_id == request_id => {
                    self.ready.store(true, Ordering::SeqCst);
                    self.emit(operation_id, generation, ExecutorEventKind::Completed, None);
                    return Ok(result);
                }
                ExecutorMessage::Failed {
                    request_id: response_id,
                    error,
                } if response_id == request_id => {
                    self.emit(
                        operation_id,
                        generation,
                        ExecutorEventKind::Error,
                        Some(error.clone()),
                    );
                    return Err(RequestFailure::fatal(anyhow::anyhow!(
                        "embedding executor failed: {error}"
                    )));
                }
                _ => {
                    // A response for a different request id means the
                    // parent/child streams are permanently misaligned.
                    // Restart the child so the next request starts from a
                    // clean protocol state instead of failing forever.
                    self.restart_child(
                        &mut state,
                        operation_id,
                        ExecutorEventKind::Exited,
                        format!(
                            "executor protocol desynchronized: expected request id {request_id}"
                        ),
                    )
                    .await?;
                    return Err(RequestFailure::retriable(anyhow::anyhow!(
                        "embedding executor generation {generation} returned a mismatched request id and restarted"
                    )));
                }
            }
        }
    }

    /// Stops the current child and installs a fresh generation. The child is
    /// taken out of the state first so the caller's stream handle and this
    /// mutation never alias. If stopping fails, the state is left cleared —
    /// dropping the child kills it via `kill_on_drop` — so the next request
    /// always spawns a clean executor.
    async fn restart_child(
        &self,
        state: &mut Option<ChildState>,
        operation_id: Option<&str>,
        kind: ExecutorEventKind,
        message: String,
    ) -> std::result::Result<(), RequestFailure> {
        let mut child = match state.take() {
            Some(child) => child,
            None => {
                *state = Some(self.spawn_child().await.map_err(RequestFailure::retriable)?);
                return Ok(());
            }
        };
        if let Err(error) = self
            .stop_child(&mut child, operation_id, kind, message)
            .await
        {
            drop(child);
            return Err(RequestFailure::retriable(
                error.context("failed to stop embedding executor"),
            ));
        }
        *state = Some(
            self.spawn_child()
                .await
                .map_err(RequestFailure::retriable)?,
        );
        Ok(())
    }

    pub async fn terminate_idle_executor(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(mut child) = state.take() {
            self.stop_child(
                &mut child,
                None,
                ExecutorEventKind::Exited,
                "executor terminated by supervisor".to_owned(),
            )
            .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl EmbeddingService for SupervisedEmbeddingService {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        match self
            .request(
                None,
                ExecutorCommand::Embed {
                    part_index: 0,
                    text: text.to_owned(),
                },
            )
            .await?
        {
            ExecutorResult::Embedding { embedding } => Ok(embedding),
            _ => anyhow::bail!("embedding executor returned the wrong result type"),
        }
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
        match self
            .request(None, ExecutorCommand::EmbedBatch { texts })
            .await?
        {
            ExecutorResult::Batch { embeddings } => Ok(embeddings),
            _ => anyhow::bail!("embedding executor returned the wrong result type"),
        }
    }

    async fn plan(&self, text: &str) -> Result<Vec<EmbeddingPlanPart>> {
        match self
            .request(
                None,
                ExecutorCommand::Plan {
                    text: text.to_owned(),
                },
            )
            .await?
        {
            ExecutorResult::Plan { parts } => Ok(parts),
            _ => anyhow::bail!("embedding executor returned the wrong result type"),
        }
    }

    async fn plan_for_operation(
        &self,
        operation_id: &str,
        text: &str,
    ) -> Result<Vec<EmbeddingPlanPart>> {
        match self
            .request(
                Some(operation_id),
                ExecutorCommand::Plan {
                    text: text.to_owned(),
                },
            )
            .await?
        {
            ExecutorResult::Plan { parts } => Ok(parts),
            _ => anyhow::bail!("embedding executor returned the wrong result type"),
        }
    }

    async fn embed_for_operation(
        &self,
        operation_id: &str,
        part_index: usize,
        text: &str,
    ) -> Result<Embedding> {
        match self
            .request(
                Some(operation_id),
                ExecutorCommand::Embed {
                    part_index,
                    text: text.to_owned(),
                },
            )
            .await?
        {
            ExecutorResult::Embedding { embedding } => Ok(embedding),
            _ => anyhow::bail!("embedding executor returned the wrong result type"),
        }
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    fn subscribe_executor_events(&self) -> Option<broadcast::Receiver<ExecutorEvent>> {
        Some(self.events_tx.subscribe())
    }

    fn executor_snapshot(&self) -> Option<ExecutorSnapshot> {
        Some(ExecutorSnapshot {
            generation: self.current_generation.load(Ordering::SeqCst),
            progress_seq: self.progress_seq.load(Ordering::SeqCst),
            exit_count: self.exit_count.load(Ordering::SeqCst),
            last_exit: self.last_exit.lock().expect("executor exit mutex").clone(),
            error: self
                .last_error
                .lock()
                .expect("executor error mutex")
                .clone(),
        })
    }

    async fn prepare_operation(
        &self,
        operation_id: &str,
        previous: &ExecutorSnapshot,
    ) -> Result<()> {
        self.next_generation
            .fetch_max(previous.generation.saturating_add(1), Ordering::SeqCst);
        let mut state = self.state.lock().await;
        if let Some(child) = state.as_mut()
            && child.generation < previous.generation
        {
            self.stop_child(
                child,
                None,
                ExecutorEventKind::Exited,
                "executor generation preceded durable ledger generation".to_owned(),
            )
            .await?;
            *state = None;
        }
        drop(state);

        self.operation_baselines
            .lock()
            .expect("operation baseline mutex")
            .entry(operation_id.to_owned())
            .or_insert_with(|| OperationBaseline {
                persisted_exit_count: previous.exit_count,
                session_exit_start: self.exit_count.load(Ordering::SeqCst),
                last_exit: previous.last_exit.clone(),
                error: previous.error.clone(),
            });
        Ok(())
    }

    fn executor_snapshot_for_operation(&self, operation_id: &str) -> Option<ExecutorSnapshot> {
        let baseline = self
            .operation_baselines
            .lock()
            .expect("operation baseline mutex")
            .get(operation_id)
            .cloned()?;
        Some(ExecutorSnapshot {
            generation: self.current_generation.load(Ordering::SeqCst),
            progress_seq: self.progress_seq.load(Ordering::SeqCst),
            exit_count: baseline.persisted_exit_count
                + self
                    .exit_count
                    .load(Ordering::SeqCst)
                    .saturating_sub(baseline.session_exit_start),
            last_exit: baseline.last_exit,
            error: baseline.error,
        })
    }
}

pub async fn run_embedding_executor() -> Result<()> {
    #[cfg(debug_assertions)]
    let fixture = std::env::var("SURREAL_EXECUTOR_FIXTURE").is_ok_and(|value| value == "1");
    #[cfg(not(debug_assertions))]
    let fixture = false;

    let service: Arc<dyn EmbeddingService> = if fixture {
        Arc::new(FixtureEmbeddingService)
    } else {
        let config = crate::config::Config::from_env()?;
        Arc::from(
            surreal_memory::embeddings::create_embedding_service(config.embedding_provider).await?,
        )
    };
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let stdout = tokio::io::stdout();
    let mut writer = BufWriter::new(stdout);
    while let Some(line) = lines.next_line().await? {
        let request: ExecutorRequest = serde_json::from_str(&line)?;
        write_message(
            &mut writer,
            &ExecutorMessage::Progress {
                request_id: request.request_id,
                phase: "accepted".to_owned(),
            },
        )
        .await?;

        let mut heartbeat = tokio::time::interval(Duration::from_millis(250));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let work = execute_request(Arc::clone(&service), &request.command);
        tokio::pin!(work);
        loop {
            tokio::select! {
                result = &mut work => {
                    let message = match result {
                        Ok(result) => ExecutorMessage::Completed {
                            request_id: request.request_id,
                            result,
                        },
                        Err(error) => ExecutorMessage::Failed {
                            request_id: request.request_id,
                            error: format!("{error:#}"),
                        },
                    };
                    write_message(&mut writer, &message).await?;
                    break;
                }
                _ = heartbeat.tick() => {
                    write_message(
                        &mut writer,
                        &ExecutorMessage::Progress {
                            request_id: request.request_id,
                            phase: "working".to_owned(),
                        },
                    ).await?;
                }
            }
        }
    }
    Ok(())
}

async fn execute_request(
    service: Arc<dyn EmbeddingService>,
    command: &ExecutorCommand,
) -> Result<ExecutorResult> {
    match command {
        ExecutorCommand::Plan { text } => Ok(ExecutorResult::Plan {
            parts: service.plan(text).await?,
        }),
        ExecutorCommand::Embed { text, .. } => Ok(ExecutorResult::Embedding {
            embedding: service.embed(text).await?,
        }),
        ExecutorCommand::EmbedBatch { texts } => Ok(ExecutorResult::Batch {
            embeddings: service.embed_batch(texts.clone()).await?,
        }),
    }
}

async fn write_message(
    writer: &mut BufWriter<tokio::io::Stdout>,
    message: &ExecutorMessage,
) -> Result<()> {
    writer.write_all(&serde_json::to_vec(message)?).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

struct FixtureEmbeddingService;

#[async_trait]
impl EmbeddingService for FixtureEmbeddingService {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        exit_fixture_once(text)?;
        if std::env::var("SURREAL_EXECUTOR_FREEZE_ON").as_deref() == Ok(text) {
            std::thread::sleep(Duration::from_secs(30));
        }
        if std::env::var("SURREAL_EXECUTOR_SLOW_ON").as_deref() == Ok(text) {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        Ok(vec![text.len() as f32 + 1.0, 1.0])
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>> {
        Ok(texts
            .iter()
            .map(|text| vec![text.len() as f32 + 1.0, 1.0])
            .collect())
    }

    async fn plan(&self, text: &str) -> Result<Vec<EmbeddingPlanPart>> {
        let contents = if text.contains("three-part") {
            vec!["alpha", "beta", "gamma"]
        } else {
            vec![text]
        };
        Ok(contents
            .into_iter()
            .enumerate()
            .map(|(part_index, content)| EmbeddingPlanPart {
                part_index,
                token_start: part_index * 3,
                token_end: (part_index + 1) * 3,
                token_count: 3,
                token_hash: format!("fixture-{part_index}-{content}"),
                content: content.to_owned(),
            })
            .collect())
    }

    fn dimensions(&self) -> usize {
        2
    }
}

fn exit_fixture_once(text: &str) -> Result<()> {
    let Some(marker_dir) = std::env::var_os("SURREAL_EXECUTOR_EXIT_MARKER_DIR") else {
        return Ok(());
    };
    let requested = std::env::var("SURREAL_EXECUTOR_EXIT_ON").unwrap_or_default();
    if !requested.split(',').any(|candidate| candidate == text) {
        return Ok(());
    }
    let marker = PathBuf::from(marker_dir).join(format!("{text}.exited"));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
    {
        Ok(file) => {
            file.sync_all()?;
            std::process::exit(91);
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}
