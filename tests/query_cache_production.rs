//! Final local integration gate, never an ignored/skipped unit fixture.
//!
//! Select only this target after the phase implementation/generation boundary:
//! `cargo test -p surreal-memory-server --test query_cache_production -- --nocapture`
//! The coordinator supplies every LDD_* input documented in Inputs::read below,
//! serializes Cargo, and maps a BLOCKED marker/child exit 2 to its own exit 2.
//! This target starts real server binaries, a real SurrealDB process/container,
//! and actual MLX/Candle workers. It never accesses an installed API or :23001.
use anyhow::{Context, Result, bail, ensure};
use reqwest::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};
use tokio::{io::{AsyncBufReadExt, BufReader}, task::JoinSet};

const MODEL: &str = "BAAI/bge-small-en-v1.5";
const REVISION: &str = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a";
const CONTENT: &str = "Restore the signed journal, project replica, identity keys and durable receipts together before recovering phase progress.";
const SAMPLES: usize = 30;
const WRITERS: usize = 8;
const QUERY_BUDGET: Duration = Duration::from_millis(2800);

struct Inputs {
    scratch: PathBuf,
    evidence: PathBuf,
    candidate: PathBuf,
    prior: PathBuf,
    candidate_source: String,
    prior_source: String,
    executor: PathBuf,
    snapshot: PathBuf,
    surreal: Option<PathBuf>,
    docker: Option<PathBuf>,
    image: Option<String>,
    team: Option<TeamInputs>,
}

struct TeamInputs {
    scenario: PathBuf,
    node: PathBuf,
    installed: PathBuf,
    runtime: PathBuf,
    control: PathBuf,
    model_input: PathBuf,
}

impl TeamInputs {
    fn read_optional() -> Result<Option<Self>> {
        let names = ["LDD_TEAM_SCENARIO_BIN", "LDD_TEAM_NODE_BIN", "LDD_TEAM_INSTALLED_ROOT",
            "LDD_TEAM_RUNTIME_BIN", "LDD_TEAM_CONTROL_HOST_BIN", "LDD_TEAM_MODEL_INPUT"];
        if names.iter().all(|name| std::env::var_os(name).is_none()) { return Ok(None); }
        // A partially supplied hook is a blocked prerequisite, never a skipped
        // team gate. The coordinator owns whether this additional gate is required.
        Ok(Some(Self {
            scenario: declared_path("LDD_TEAM_SCENARIO_BIN", false)?,
            node: declared_path("LDD_TEAM_NODE_BIN", false)?,
            installed: declared_path("LDD_TEAM_INSTALLED_ROOT", true)?,
            runtime: declared_path("LDD_TEAM_RUNTIME_BIN", false)?,
            control: declared_path("LDD_TEAM_CONTROL_HOST_BIN", false)?,
            model_input: declared_path("LDD_TEAM_MODEL_INPUT", false)?,
        }))
    }
}

fn required(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("BLOCKED: explicit {name} is required"))
}

fn declared_path(name: &str, directory: bool) -> Result<PathBuf> {
    let p = PathBuf::from(required(name)?);
    ensure!(p.is_absolute(), "BLOCKED: {name} must be absolute");
    let p = fs::canonicalize(p).with_context(|| format!("BLOCKED: resolve {name}"))?;
    ensure!(if directory { p.is_dir() } else { p.is_file() }, "BLOCKED: unavailable {name}");
    Ok(p)
}

fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = [0_u8; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 { break; }
        hasher.update(&bytes[..n]);
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

impl Inputs {
    fn read() -> Result<Self> {
        ensure!(cfg!(unix), "BLOCKED: this process-group gate requires a Unix host");
        let candidate_source = required("LDD_SERVER_SOURCE_ID")?;
        let prior_source = required("LDD_PRIOR_SOURCE_ID")?;
        for identity in [&candidate_source, &prior_source] {
            ensure!([40, 64].contains(&identity.len()) && identity.bytes().all(|b| b.is_ascii_hexdigit()),
                "BLOCKED: source identities must be recorded commit/tree or SHA-256 provenance, never invented labels");
        }
        let snapshot = declared_path("LDD_MODEL_SNAPSHOT", true)?;
        ensure!(snapshot.file_name().and_then(|p| p.to_str()) == Some(REVISION), "BLOCKED: model snapshot revision must be {REVISION}");
        for name in ["config.json", "tokenizer.json", "model.safetensors"] {
            ensure!(snapshot.join(name).is_file(), "BLOCKED: pinned model asset missing: {name}");
        }
        let surreal = if std::env::var_os("LDD_SURREAL_BIN").is_some() { Some(declared_path("LDD_SURREAL_BIN", false)?) } else { None };
        let docker = if surreal.is_none() && std::env::var_os("LDD_DOCKER_BIN").is_some() { Some(declared_path("LDD_DOCKER_BIN", false)?) } else { None };
        ensure!(surreal.is_some() || docker.is_some(), "BLOCKED: declare LDD_SURREAL_BIN or LDD_DOCKER_BIN for real SurrealDB 3.3.0");
        let image = if docker.is_some() { Some(required("LDD_SURREAL_IMAGE")?) } else { None };
        let scratch = declared_path("LDD_SCRATCH_ROOT", true)?;
        for forbidden in ["/Users/gqadonis/.codex", "/Users/gqadonis/.claude", "/Users/gqadonis/.cortex", "/Users/gqadonis/.prometheus"] {
            ensure!(!scratch.starts_with(forbidden), "BLOCKED: scratch root overlaps a real runtime home");
        }
        Ok(Self { scratch, evidence: declared_path("LDD_EVIDENCE_DIR", true)?,
            candidate: declared_path("LDD_SERVER_BIN", false)?, prior: declared_path("LDD_PRIOR_SERVER_BIN", false)?,
            candidate_source, prior_source, executor: declared_path("LDD_MLX_EXECUTOR_BIN", false)?, snapshot, surreal, docker, image,
            team: TeamInputs::read_optional()? })
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn clean_command(binary: &Path, cwd: &Path, env: &BTreeMap<String, String>) -> Command {
    let mut command = Command::new(binary);
    command.env_clear().envs(env).current_dir(cwd).stdin(Stdio::null());
    #[cfg(unix)] {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

fn signal(pid: u32, name: &str, group: bool) {
    let target = if group { format!("-{pid}") } else { pid.to_string() };
    let _ = Command::new("/bin/kill").args([name, "--", &target]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

struct ProcessGuard { child: Child }
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let pid = self.child.id();
        signal(pid, "-TERM", true);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() { break; }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Reap the parent and its still-owned executor group even after a clean
        // parent exit. Never find/terminate processes by a global name.
        signal(pid, "-KILL", true);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct ContainerGuard { binary: PathBuf, name: String, cwd: PathBuf, env: BTreeMap<String, String> }
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = clean_command(&self.binary, &self.cwd, &self.env).args(["rm", "--force", &self.name]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

struct Database {
    endpoint: String,
    identity: Value,
    _process: Option<ProcessGuard>,
    _container: Option<ContainerGuard>,
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    ensure!(port != 23001, "reserved live port must never be used");
    Ok(port)
}

fn child_env(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    for (key, sub) in [("HOME", "home"), ("CODEX_HOME", "codex"), ("CLAUDE_CONFIG_DIR", "claude"),
        ("CORTEX_DATA_DIR", "cortex"), ("TMPDIR", "tmp"), ("HF_HOME", "models"),
        ("MODEL_CACHE_DIR", "models"), ("HF_HUB_CACHE", "models/hub"), ("DOCKER_CONFIG", "docker"),
        ("PROMETHEUS_LEARNING_LOG_DIR", "learning/log"), ("PROMETHEUS_LEARNING_OUTBOX", "learning/outbox"),
        ("PROMETHEUS_LEARNING_INDEX", "learning/index"), ("PROMETHEUS_LEARNING_STORE_ROOT", "learning/store")] {
        let path = root.join(sub); fs::create_dir_all(&path)?;
        env.insert(key.into(), path.to_string_lossy().into_owned());
    }
    env.insert("PROMETHEUS_LEARNING_LOG".into(), root.join("learning/lessons.jsonl").to_string_lossy().into_owned());
    env.insert("HF_HUB_OFFLINE".into(), "1".into());
    env.insert("HF_HUB_DISABLE_TELEMETRY".into(), "1".into());
    env.insert("EMBEDDING_PROVIDER".into(), "local".into());
    env.insert("LOCAL_EMBEDDING_MODEL".into(), MODEL.into());
    env.insert("LOCAL_EMBEDDING_MODEL_REVISION".into(), REVISION.into());
    env.insert("LOCAL_EMBEDDING_DIMENSIONS".into(), "384".into());
    env.insert("LOCAL_EMBEDDING_DEVICE".into(), "cpu".into());
    env.insert("EMBEDDING_WARMUP".into(), "true".into());
    env.insert("MCP_STDIO".into(), "false".into());
    env.insert("API_HOST".into(), "127.0.0.1".into());
    env.insert("SURREAL_EXECUTOR_STARTUP_MS".into(), "300000".into());
    env.insert("SURREAL_EXECUTOR_WATCHDOG_MS".into(), "30000".into());
    env.insert("SURREAL_QUERY_TIMEOUT_MS".into(), "10000".into());
    env.insert("RUST_LOG".into(), "surreal_memory_server=info,surreal_memory=info,surreal=warn".into());
    if let Ok(host) = std::env::var("LDD_DOCKER_HOST") {
        ensure!(host.starts_with("unix:///"), "BLOCKED: LDD_DOCKER_HOST must name an explicit local Unix socket");
        env.insert("DOCKER_HOST".into(), host);
    }
    Ok(env)
}

fn copy_model(inputs: &Inputs, root: &Path) -> Result<Value> {
    let target = root.join("models/hub/models--BAAI--bge-small-en-v1.5/snapshots").join(REVISION);
    fs::create_dir_all(&target)?;
    let mut files = Vec::new();
    // Copy only declared model assets, never HF account/token/config state.
    for name in ["config.json", "tokenizer.json", "model.safetensors", "tokenizer_config.json", "special_tokens_map.json", "vocab.txt", "modules.json", "sentence_bert_config.json", "config_sentence_transformers.json"] {
        let from = inputs.snapshot.join(name);
        if from.is_file() {
            fs::copy(&from, target.join(name))?;
            files.push(json!({"name":name,"sha256":digest(&from)?}));
        }
    }
    Ok(json!({"model":MODEL,"revision":REVISION,"dimensions":384,"readonlyInput":inputs.snapshot,"scratchSnapshot":target,"files":files}))
}

async fn start_database(inputs: &Inputs, root: &Path, env: &BTreeMap<String, String>, client: &Client) -> Result<Database> {
    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");
    let mut database = if let Some(binary) = &inputs.surreal {
        let version = clean_command(binary, root, env).arg("version").output().context("BLOCKED: SurrealDB version command")?;
        let text = String::from_utf8_lossy(&version.stdout);
        ensure!(version.status.success() && text.contains("3.3.0"), "BLOCKED: require actual SurrealDB 3.3.0, observed {text}");
        let child = clean_command(binary, root, env)
            .args(["start", "--no-banner", "--unauthenticated", "--allow-all", "--bind", &bind, "memory"])
            .stdout(File::create(inputs.evidence.join("cache-surreal.stdout"))?)
            .stderr(File::create(inputs.evidence.join("cache-surreal.stderr"))?).spawn().context("BLOCKED: start real SurrealDB")?;
        Database { endpoint: format!("ws://{bind}"), identity: json!({"mode":"server-memory","binary":binary,"binarySha256":digest(binary)?,"version":text.trim(),"bind":bind}), _process: Some(ProcessGuard { child }), _container: None }
    } else {
        let binary = inputs.docker.as_ref().context("BLOCKED: Docker binary")?;
        let image = inputs.image.as_ref().context("BLOCKED: pinned SurrealDB image")?;
        let name = format!("ldd-cache-{}", uuid::Uuid::new_v4().simple());
        let publish = format!("127.0.0.1:{port}:8000");
        let guard = ContainerGuard { binary: binary.clone(), name: name.clone(), cwd: root.to_owned(), env: env.clone() };
        let result = clean_command(binary, root, env).args(["run", "--detach", "--rm", "--pull=never", "--name", &name,
            "--label", "prometheus.ldd.scratch=true", "--publish", &publish, image, "start", "--no-banner", "--unauthenticated", "--allow-all", "--bind", "0.0.0.0:8000", "memory"])
            .output().context("BLOCKED: start preprovisioned real SurrealDB container")?;
        ensure!(result.status.success(), "BLOCKED: Docker/real pinned SurrealDB image unavailable: {}", String::from_utf8_lossy(&result.stderr));
        let image_id = clean_command(binary, root, env).args(["inspect", "--format", "{{.Image}}", &name]).output()?;
        let version = clean_command(binary, root, env).args(["exec", &name, "/surreal", "version"]).output()?;
        ensure!(version.status.success() && String::from_utf8_lossy(&version.stdout).contains("3.3.0"), "BLOCKED: container must actually run SurrealDB 3.3.0");
        Database { endpoint: format!("ws://{bind}"), identity: json!({"mode":"server-memory-container","container":name,"image":image,
            "imageId":String::from_utf8_lossy(&image_id.stdout).trim(),"version":String::from_utf8_lossy(&version.stdout).trim(),"bind":bind}), _process: None, _container: Some(guard) }
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if client.get(format!("http://{bind}/health")).timeout(Duration::from_secs(1)).send().await.is_ok_and(|r| r.status().is_success()) { break; }
        if let Some(process) = database._process.as_mut() {
            ensure!(process.child.try_wait()?.is_none(), "BLOCKED: scratch SurrealDB exited during startup");
        }
        ensure!(Instant::now() < deadline, "BLOCKED: real SurrealDB did not become ready");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(database)
}

struct Server { process: ProcessGuard, base: String, health: Value }
async fn start_server(inputs: &Inputs, binary: &Path, label: &str, backend: &str, capacity: usize,
    root: &Path, environment: &BTreeMap<String, String>, endpoint: &str, client: &Client) -> Result<Server> {
    let cwd = root.join(label); fs::create_dir_all(&cwd)?;
    let mut env = environment.clone();
    let port = free_port()?;
    env.insert("API_PORT".into(), port.to_string());
    env.insert("SURREAL_MODE".into(), "server".into());
    env.insert("SURREAL_ENDPOINT".into(), endpoint.into());
    env.insert("SURREAL_NAMESPACE".into(), format!("ldd_{}", uuid::Uuid::new_v4().simple()));
    env.insert("SURREAL_DATABASE".into(), "production_cache".into());
    env.insert("LOCAL_EMBEDDING_BACKEND".into(), backend.into());
    env.insert("LOCAL_EMBEDDING_EXECUTOR".into(), inputs.executor.to_string_lossy().into_owned());
    env.insert("SURREAL_MEMORY_QUERY_EMBED_CACHE".into(), capacity.to_string());
    let child = clean_command(binary, &cwd, &env).stdout(File::create(inputs.evidence.join(format!("cache-{label}.stdout")))?)
        .stderr(File::create(inputs.evidence.join(format!("cache-{label}.stderr")))?).spawn().with_context(|| format!("BLOCKED: start {label} production binary"))?;
    let mut server = Server { process: ProcessGuard { child }, base: format!("http://127.0.0.1:{port}"), health: Value::Null };
    let deadline = Instant::now() + Duration::from_secs(330);
    loop {
        if let Ok(response) = client.get(format!("{}/ready", server.base)).timeout(Duration::from_secs(1)).send().await {
            if response.status().is_success() {
                let body: Value = response.json().await?;
                if body["search_ready"] == true && body["capabilities"]["model_executor"] == true { break; }
            }
        }
        ensure!(server.process.child.try_wait()?.is_none(), "BLOCKED: {label} exited before real model/database readiness; inspect its owned logs");
        ensure!(Instant::now() < deadline, "BLOCKED: {label} real local model/database readiness unavailable");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    server.health = client.get(format!("{}/health", server.base)).send().await?.error_for_status()?.json().await?;
    Ok(server)
}

async fn worker_identity(binary: &Path, backend: &str, environment: &BTreeMap<String, String>, root: &Path) -> Result<Value> {
    let mut env = environment.clone(); env.insert("LOCAL_EMBEDDING_BACKEND".into(), backend.into());
    let mut command = clean_command(binary, root, &env);
    command.arg("embedding-executor").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut command = tokio::process::Command::from(command); command.kill_on_drop(true);
    let mut child = command.spawn().context("BLOCKED: actual worker identity process")?;
    let result: Result<Value> = async {
        let stdout = child.stdout.take().context("worker stdout")?;
        let mut lines = BufReader::new(stdout).lines();
        let line = tokio::time::timeout(Duration::from_secs(300), lines.next_line()).await
            .context("BLOCKED: actual worker readiness timeout")??.context("BLOCKED: actual worker exited before ready")?;
        let ready: Value = serde_json::from_str(&line)?;
        ensure!(ready["message"] == "ready" && ready["model_id"] == MODEL && ready["model_revision"] == REVISION && ready["dimensions"] == 384,
            "BLOCKED: actual worker identity differs from approved immutable configuration: {ready}");
        ensure!(ready["backend"] == backend, "actual backend identity mismatch");
        ensure!(ready["cache_namespace"].as_str().is_some_and(|s| !s.is_empty()), "actual worker did not identify its cache namespace");
        Ok(ready)
    }.await;
    let _ = child.kill().await; let _ = child.wait().await;
    result
}

async fn optional_stats(client: &Client, server: &Server) -> Result<Option<Value>> {
    let response = client.get(format!("{}/api/v2/operations/stats", server.base)).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND { return Ok(None); }
    let body: Value = response.error_for_status()?.json().await?;
    if body.get("query_embed_cache").is_none_or(Value::is_null) { return Ok(None); }
    ensure!(body["query_embed_cache"].is_object(), "invalid historical cache stats object: {body}");
    Ok(Some(body["query_embed_cache"].clone()))
}

async fn stats(client: &Client, server: &Server) -> Result<Value> {
    let body: Value = client.get(format!("{}/api/v2/operations/stats", server.base)).send().await?.error_for_status()?.json().await?;
    ensure!(body["query_embed_cache"].is_object(), "successful stats omitted supported production cache counters: {body}");
    Ok(body["query_embed_cache"].clone())
}
fn number(stats: &Value, name: &str) -> Result<u64> { stats[name].as_u64().with_context(|| format!("missing counter {name}: {stats}")) }

async fn search(client: &Client, base: &str, query: &str, budget: Duration) -> Result<Value> {
    Ok(client.post(format!("{base}/api/v1/search")).timeout(budget)
        .json(&json!({"query":query,"user_id":"reader","limit":3})).send().await?.error_for_status()?.json().await?)
}
async fn add(client: &Client, base: &str, content: &str, user: &str) -> Result<Value> {
    Ok(client.post(format!("{base}/api/v1/memory/")).json(&json!({"content":content,"user_id":user}))
        .send().await?.error_for_status()?.json().await?)
}
fn relevant(results: &Value, id: &str) -> bool {
    results.as_array().is_some_and(|rows| rows.iter().any(|row| row["id"].as_str() == Some(id)))
}

async fn cache_contract(client: &Client, server: &Server) -> Result<Value> {
    let initial = stats(client, server).await?;
    ensure!(initial == json!({"capacity":512,"entries":0,"hits":0,"misses":0}), "fresh cache not empty: {initial}");
    let seed = add(client, &server.base, CONTENT, "reader").await?;
    let seed_id = seed["id"].as_str().context("real seed id")?.to_owned();
    add(client, &server.base, CONTENT, "reader").await?; // actual duplicate detection
    client.put(format!("{}/api/v1/memory/{seed_id}", server.base)).json(&json!({"content":CONTENT}))
        .send().await?.error_for_status()?;
    ensure!(stats(client, server).await? == initial, "create/update/dedup changed query-cache counters");
    let first = search(client, &server.base, CONTENT, Duration::from_secs(30)).await?;
    ensure!(relevant(&first, &seed_id), "real model/search did not recall its scoped seed");
    let normalized = format!("  {}  ", CONTENT.replace(' ', "  "));
    search(client, &server.base, &normalized, Duration::from_secs(30)).await?;
    let repeat = stats(client, server).await?;
    ensure!(number(&repeat,"misses")? == 1 && number(&repeat,"hits")? == 1 && number(&repeat,"entries")? == 1, "normalization/reuse: {repeat}");
    search(client, &server.base, "caf\u{e9} canonical text", Duration::from_secs(30)).await?;
    search(client, &server.base, "cafe\u{301} canonical text", Duration::from_secs(30)).await?;
    let unicode = stats(client, server).await?;
    ensure!(number(&unicode,"misses")? == number(&repeat,"misses")? + 1 && number(&unicode,"hits")? == number(&repeat,"hits")? + 1,
        "real NFC-equivalent queries did not share a key: {unicode}");
    let query = format!("Concurrent exact-key query {}", uuid::Uuid::new_v4());
    let mut tasks = JoinSet::new();
    let barrier = Arc::new(tokio::sync::Barrier::new(WRITERS));
    for _ in 0..WRITERS {
        let client = client.clone(); let base = server.base.clone(); let query = query.clone(); let barrier = Arc::clone(&barrier);
        tasks.spawn(async move { barrier.wait().await; search(&client, &base, &query, Duration::from_secs(30)).await });
    }
    while let Some(result) = tasks.join_next().await { result??; }
    let coalesced = stats(client, server).await?;
    ensure!(number(&coalesced,"misses")? == number(&unicode,"misses")? + 1 && number(&coalesced,"hits")? == number(&unicode,"hits")? + 7,
        "real concurrent reuse/coalescing counters: {coalesced}");
    // These real model inputs exceed the pinned MLX model's token capacity.
    // No fake provider or executor response supplies the failure.
    let invalid = "unbounded ".repeat(4096);
    for _ in 0..2 {
        let response = client.post(format!("{}/api/v1/search", server.base)).json(&json!({"query":invalid,"user_id":"reader","limit":3})).send().await?;
        ensure!(response.status().is_server_error(), "actual MLX oversized input unexpectedly succeeded");
        let failure: Value = response.json().await?;
        ensure!(failure["error"].as_str().is_some_and(|s| s.contains("input_too_long")), "unrelated failure is not model rejection evidence: {failure}");
    }
    let failed = stats(client, server).await?;
    ensure!(number(&failed,"misses")? == number(&coalesced,"misses")? + 2 && number(&failed,"hits")? == number(&coalesced,"hits")? && number(&failed,"entries")? == number(&coalesced,"entries")?, "failed producer accounting/caching: {failed}");
    Ok(json!({"seedId":seed_id,"initial":initial,"repeat":repeat,"unicodeNfc":unicode,"coalesced":coalesced,"failed":failed,"concurrentCallers":WRITERS,
        "coalescingObservation":"one actual producer, seven successful reusers; scheduling may include already-completed cache hits"}))
}

struct RestoreDirectory { from: PathBuf, to: PathBuf }
impl Drop for RestoreDirectory { fn drop(&mut self) { let _ = fs::rename(&self.from, &self.to); } }

async fn worker_failure(client: &Client, server: &Server, root: &Path) -> Result<Value> {
    let before = stats(client, server).await?;
    let snapshot = root.join("models/hub/models--BAAI--bge-small-en-v1.5/snapshots").join(REVISION);
    let unavailable = root.join("temporarily-unavailable-model");
    fs::rename(&snapshot, &unavailable)?;
    let restore = RestoreDirectory { from: unavailable, to: snapshot };
    let pid = server.process.child.id().to_string();
    let children = Command::new("/usr/bin/pgrep").args(["-P", &pid]).output()?;
    ensure!(children.status.success(), "no actual owned model worker found");
    let mut killed = Vec::new();
    for child in String::from_utf8_lossy(&children.stdout).split_whitespace() {
        let child_pid: u32 = child.parse()?;
        let parent = Command::new("/bin/ps").args(["-o", "ppid=", "-p", child]).output()?;
        ensure!(String::from_utf8_lossy(&parent.stdout).trim() == pid, "executor PID ownership changed; refuse termination");
        signal(child_pid, "-KILL", false); killed.push(child_pid);
    }
    let query = format!("owned-worker-loss {}", uuid::Uuid::new_v4());
    let response = client.post(format!("{}/api/v1/search", server.base)).timeout(Duration::from_secs(90))
        .json(&json!({"query":query,"user_id":"reader","limit":3})).send().await?;
    ensure!(response.status().is_server_error(), "unavailable actual worker model unexpectedly succeeded");
    let failure: Value = response.json().await?;
    let failed = stats(client, server).await?;
    ensure!(number(&failed,"misses")? == number(&before,"misses")? + 1 && number(&failed,"hits")? == number(&before,"hits")? && number(&failed,"entries")? == number(&before,"entries")?, "worker loss changed producer accounting incorrectly: {failed}");
    drop(restore);
    search(client, &server.base, &query, Duration::from_secs(330)).await?;
    search(client, &server.base, &query, Duration::from_secs(30)).await?;
    let recovered = stats(client, server).await?;
    ensure!(number(&recovered,"misses")? == number(&failed,"misses")? + 1 && number(&recovered,"hits")? == number(&failed,"hits")? + 1, "actual worker recovery did not retry and cache successfully: {recovered}");
    Ok(json!({"before":before,"killedOwnedWorkers":killed,"failure":failure,"failed":failed,"recovered":recovered,
        "invariant":"cache misses count producer invocations, not supervisor-internal spawn/retry attempts"}))
}

fn percentile(samples: &[f64], fraction: f64) -> f64 {
    let mut sorted = samples.to_vec(); sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() - 1) as f64 * fraction).ceil() as usize]
}

async fn measure(client: &Client, server: &Server, seed: &str, loaded: bool) -> Result<Value> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut writers = JoinSet::new();
    let barrier = Arc::new(tokio::sync::Barrier::new(WRITERS + 1));
    if loaded {
        for writer in 0..WRITERS {
            let client = client.clone(); let base = server.base.clone(); let stop = Arc::clone(&stop); let barrier = Arc::clone(&barrier);
            writers.spawn(async move {
                barrier.wait().await;
                let mut attempted = 0_u64; let mut successful = 0_u64;
                while !stop.load(Ordering::Relaxed) {
                    attempted += 1;
                    let content = format!("Writer {writer} observation {attempted} unique {}: astronomy soil irrigation and durable background memory capture.", uuid::Uuid::new_v4());
                    if add(&client, &base, &content, &format!("writer-{writer}")).await.is_ok() { successful += 1; }
                }
                (attempted, successful)
            });
        }
        barrier.wait().await;
        // Let all eight real write requests reach the service before queries.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut latencies = Vec::new(); let mut timeouts = 0; let mut errors = 0; let mut recalled = 0;
    for _ in 0..SAMPLES {
        let started = Instant::now();
        match search(client, &server.base, CONTENT, QUERY_BUDGET).await {
            Ok(rows) => { if relevant(&rows, seed) { recalled += 1; } }
            Err(error) => { if error.downcast_ref::<reqwest::Error>().is_some_and(|e| e.is_timeout()) { timeouts += 1; } else { errors += 1; } }
        }
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    stop.store(true, Ordering::Relaxed);
    let mut writer_counts = Vec::new();
    while let Some(result) = writers.join_next().await { let (attempted, successful) = result?; writer_counts.push(json!({"attempted":attempted,"successful":successful})); }
    if loaded { ensure!(writer_counts.len() == WRITERS && writer_counts.iter().all(|w| w["successful"].as_u64().unwrap_or(0) > 0), "not all eight real writers completed writes: {writer_counts:?}"); }
    Ok(json!({"samples":SAMPLES,"writers":if loaded {WRITERS} else {0},"queryDeadlineMs":QUERY_BUDGET.as_millis(),
        "p50Ms":percentile(&latencies,0.5),"p95Ms":percentile(&latencies,0.95),"timeouts":timeouts,"errors":errors,
        "relevantResults":recalled,"relevanceFraction":recalled as f64/SAMPLES as f64,"writerCounts":writer_counts,"latenciesMs":latencies}))
}

fn record(path: &Path, label: &str, value: &Value) -> Result<()> {
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", json!({"at":chrono::Utc::now().to_rfc3339(),"label":label,"value":value}))?;
    file.sync_all()?; Ok(())
}

fn preserve_team_evidence(from: &Path, to: &Path, entries: &mut Vec<Value>) -> Result<()> {
    let metadata = fs::symlink_metadata(from)?;
    ensure!(!metadata.file_type().is_symlink(), "team evidence must not traverse a symlink: {}", from.display());
    if metadata.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            preserve_team_evidence(&entry.path(), &to.join(entry.file_name()), entries)?;
        }
    } else {
        ensure!(metadata.is_file(), "team evidence is not a regular file: {}", from.display());
        // Only the scenario's explicit evidence tree is copied. Scratch identity
        // keys, request bodies and real model-input configuration are not copied.
        let mut source = File::open(from)?;
        let mut target = fs::OpenOptions::new().write(true).create_new(true).open(to)?;
        std::io::copy(&mut source, &mut target)?;
        target.sync_all()?;
        entries.push(json!({"path":to,"sha256":digest(to)?}));
    }
    Ok(())
}

async fn dispatch_team(inputs: &Inputs, server: &Server, environment: &BTreeMap<String, String>, journal: &Path) -> Result<()> {
    let Some(team) = &inputs.team else {
        record(journal,"team-dispatch",&json!({"status":"not-requested","acceptance":false,
            "reason":"No LDD_TEAM_* inputs supplied; this cache target does not satisfy the separate team gate."}))?;
        return Ok(());
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let scratch = inputs.scratch.join(format!("t-{}", &id[..12]));
    ensure!(scratch.join("control.sock").as_os_str().len() <= 100,
        "BLOCKED: LDD_SCRATCH_ROOT is too long for the real Companion Unix socket; supply a shorter private root");
    fs::create_dir(&scratch)?;
    let _scratch = Scratch(scratch.clone());
    // Keep this root empty until the actual scenario starts. Its --evidence
    // argument names a receipt FILE under a private evidence directory.
    let receipt = scratch.join("evidence/team-control.json");
    let preserved = inputs.evidence.join("team").join(&id);
    fs::create_dir_all(preserved.parent().context("team evidence parent")?)?;
    let stdout = inputs.evidence.join(format!("team-{id}.stdout"));
    let stderr = inputs.evidence.join(format!("team-{id}.stderr"));
    let argv = vec![team.scenario.to_string_lossy().into_owned(),
        "--scratch".into(), scratch.to_string_lossy().into_owned(),
        "--evidence".into(), receipt.to_string_lossy().into_owned(),
        "--installed-root".into(), team.installed.to_string_lossy().into_owned(),
        "--runtime-bin".into(), team.runtime.to_string_lossy().into_owned(),
        "--model-input".into(), team.model_input.to_string_lossy().into_owned(),
        "--memory-endpoint".into(), server.base.clone(),
        "--control-host-bin".into(), team.control.to_string_lossy().into_owned()];
    let child = clean_command(&team.node, &scratch, environment).args(&argv)
        .stdout(File::create(&stdout)?).stderr(File::create(&stderr)?).spawn()
        .context("BLOCKED: start real coordinator team scenario")?;
    let mut process = ProcessGuard { child };
    let deadline = Instant::now() + Duration::from_secs(1200);
    let status = loop {
        if let Some(status) = process.child.try_wait()? { break Some(status); }
        if Instant::now() >= deadline { break None; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    // Stop only the owned process group before reading/copying its receipts.
    drop(process);
    let mut files = Vec::new();
    if scratch.join("evidence").exists() {
        preserve_team_evidence(&scratch.join("evidence"), &preserved, &mut files)?;
    }
    record(journal,"team-dispatch",&json!({"node":team.node,"nodeSha256":digest(&team.node)?,
        "scenario":team.scenario,"scenarioSha256":digest(&team.scenario)?,"argv":argv,
        "memoryEndpoint":server.base,"exitCode":status.as_ref().and_then(|s| s.code()),
        "timedOut":status.is_none(),"stdout":stdout,"stderr":stderr,"evidence":files}))?;
    let status = status.context("actual team scenario exceeded its 20-minute process budget")?;
    if status.code() == Some(2) { bail!("BLOCKED: coordinator team scenario returned 2; preserved receipts under {}", preserved.display()); }
    ensure!(status.success(), "actual team scenario failed: {status}; preserved receipts under {}", preserved.display());
    let result: Value = serde_json::from_reader(File::open(preserved.join("team-control.json"))?)?;
    ensure!(result["result"] == "PASS", "team process returned success without a PASS receipt");
    Ok(())
}

async fn run(inputs: Inputs) -> Result<()> {
    let root = inputs.scratch.join(format!("memory-production-{}", uuid::Uuid::new_v4().simple())); fs::create_dir(&root)?;
    let _scratch = Scratch(root.clone());
    let evidence = inputs.evidence.join("query-cache-production.jsonl");
    record(&evidence,"boundary", &json!({"candidate":{"binary":inputs.candidate,"sha256":digest(&inputs.candidate)?,"sourceIdentity":inputs.candidate_source},
        "prior":{"binary":inputs.prior,"sha256":digest(&inputs.prior)?,"sourceIdentity":inputs.prior_source},
        "executor":{"binary":inputs.executor,"sha256":digest(&inputs.executor)?},"scratch":root,
        "unknownNamespaceRuntime":"unverified: no actual unidentified legacy worker supplied; no artificial provider is introduced"}))?;
    let env = child_env(&root)?;
    record(&evidence,"model", &copy_model(&inputs,&root)?)?;
    let client = Client::builder().no_proxy().timeout(Duration::from_secs(90)).build()?;
    let mlx_identity = worker_identity(&inputs.executor,"mlx",&env,&root).await?;
    let candle_identity = worker_identity(&inputs.candidate,"candle",&env,&root).await?;
    ensure!(mlx_identity["cache_namespace"] != candle_identity["cache_namespace"], "actual distinct backend/config namespaces collide");
    record(&evidence,"actual-worker-namespaces", &json!({"mlx":mlx_identity,"candleCpu":candle_identity,
        "claim":"backend/config separation; not a reproduced historical cross-model leak"}))?;
    let database = start_database(&inputs,&root,&env,&client).await?;
    record(&evidence,"database", &database.identity)?;
    {
        let server = start_server(&inputs,&inputs.candidate,"candidate-enabled","mlx",512,&root,&env,&database.endpoint,&client).await?;
        ensure!(server.health["version"] == env!("CARGO_PKG_VERSION"), "candidate binary version differs from selected target");
        let contract = cache_contract(&client,&server).await?; record(&evidence,"cache-contract",&contract)?;
        record(&evidence,"worker-failure-recovery",&worker_failure(&client,&server,&root).await?)?;
        let seed = contract["seedId"].as_str().context("contract seed")?;
        for loaded in [false,true] {
            let before = stats(&client,&server).await?;
            let measured = measure(&client,&server,seed,loaded).await?;
            let after = stats(&client,&server).await?;
            record(&evidence,if loaded {"candidate-enabled-8writer"} else {"candidate-enabled-idle"},&json!({"health":server.health,"before":before,"after":after,"measurement":measured}))?;
            ensure!(measured["timeouts"] == 0 && measured["errors"] == 0 && measured["relevantResults"] == SAMPLES, "candidate recall failed its real request/relevance budget: {measured}");
            if !loaded { ensure!(measured["p95Ms"].as_f64().unwrap_or(f64::INFINITY) <= 2000.0, "idle p95 exceeds the 2-second stop rule"); }
            ensure!(number(&after,"misses")? == number(&before,"misses")? && number(&after,"hits")? == number(&before,"hits")? + SAMPLES as u64,
                "background production writes polluted query cache or repeated queries missed: {after}");
        }
        // Run after cache measurements: real team publication/search may mutate
        // this database and its counters, and must not pollute benchmark attribution.
        dispatch_team(&inputs,&server,&env,&evidence).await?;
    }
    {
        let alternate = start_server(&inputs,&inputs.candidate,"candidate-candle-cpu","candle",512,&root,&env,&database.endpoint,&client).await?;
        let empty = stats(&client,&alternate).await?; ensure!(number(&empty,"entries")? == 0 && number(&empty,"misses")? == 0 && number(&empty,"hits")? == 0,"replacement model/config cache was not fresh");
        add(&client,&alternate.base,CONTENT,"reader").await?;
        search(&client,&alternate.base,CONTENT,Duration::from_secs(30)).await?;
        search(&client,&alternate.base,CONTENT,Duration::from_secs(30)).await?;
        let reused = stats(&client,&alternate).await?;
        ensure!(number(&reused,"misses")? == 1 && number(&reused,"hits")? == 1,"actual alternate namespace reuse: {reused}");
        record(&evidence,"replacement-backend-config",&json!({"empty":empty,"reused":reused,"health":alternate.health}))?;
    }
    for (label,binary,capacity,strict) in [("candidate-disabled",&inputs.candidate,0,true),("prior-control",&inputs.prior,512,false)] {
        let server = start_server(&inputs,binary,label,"mlx",capacity,&root,&env,&database.endpoint,&client).await?;
        let seed = add(&client,&server.base,CONTENT,"reader").await?;
        let seed = seed["id"].as_str().context("control seed")?;
        for loaded in [false,true] {
            let before = optional_stats(&client,&server).await?;
            if strict { ensure!(before.is_some(),"candidate-disabled stats unavailable"); }
            let measured = measure(&client,&server,seed,loaded).await?;
            let after = optional_stats(&client,&server).await?;
            record(&evidence,&format!("{label}-{}",if loaded {"8writer"} else {"idle"}),&json!({"health":server.health,"before":before,"after":after,"measurement":measured,
                "priorCounterAbsence":"if absent, unavailable historical wire field; never synthesized counters"}))?;
            if strict {
                let before = before.context("disabled before")?; let after = after.context("disabled after")?;
                ensure!(number(&after,"capacity")? == 0 && number(&after,"entries")? == 0 && number(&after,"hits")? == 0 && number(&after,"misses")? == number(&before,"misses")? + SAMPLES as u64,
                    "disabled cache producer accounting: {after}");
                ensure!(measured["timeouts"] == 0 && measured["errors"] == 0 && measured["relevantResults"] == SAMPLES,"disabled candidate production control failed: {measured}");
            }
        }
    }
    // Exact inputs remain unchanged; model/source ownership and installed state
    // are independent from the scratch production acceptance above.
    record(&evidence,"result", &json!({"status":"pass","acceptance":["real-production-server","real-surrealdb","immutable-worker-model-identity","backend-config-namespace-separation",
        "normalized-reuse","successful-coalescing-accounting","failed-producer-accounting","write-update-dedup-counter-isolation","actual-worker-loss-and-recovery",
        "idle-and-8writer-recall","cache-disabled-control","recorded-prior-source-control"],"unknownProviderRuntime":"unverified-source-contract-only"}))?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn query_cache_through_real_production_services() {
    let result = match Inputs::read() { Ok(inputs) => run(inputs).await, Err(error) => Err(error) };
    if let Err(error) = result {
        let message = format!("{error:#}");
        if message.contains("BLOCKED:") { eprintln!("BLOCKED: query_cache_production: {message}"); std::process::exit(2); }
        panic!("query_cache_production failed: {message}");
    }
}
