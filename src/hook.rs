//! Claude Code hook / statusline client mode.
//!
//! These subcommands talk ONLY to a running server's HTTP API
//! (`SURREAL_MEMORY_URL`, default `http://localhost:23001`), so hook startup
//! never pays for a database bootstrap. Every entry point is fail-open:
//! callers print diagnostics to stderr and still exit 0, so a down or slow
//! server can never block or break the harness.
//!
//! Implements the RECALL → REMEMBER glue from `skills/surreal-memory`:
//! `session-start` recalls project memories into the agent's context,
//! `pre-compact` / `session-end` persist a session-state memory through the
//! durable operation ledger, and `statusline` reports server health.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Default server address; override with `SURREAL_MEMORY_URL`.
const DEFAULT_BASE_URL: &str = "http://localhost:23001";

/// Agent identity written to stored session-state memories.
const AGENT_ID: &str = "claude-code";

/// SessionStart and PreCompact may take a couple of seconds. SessionEnd is
/// capped by the harness at ~3s and should stay near-instant, and the
/// statusline runs on every prompt redraw, so both get tighter budgets.
const HOOK_TIMEOUT: Duration = Duration::from_millis(2500);
const SESSION_END_TIMEOUT: Duration = Duration::from_millis(900);
const STATUSLINE_TIMEOUT: Duration = Duration::from_millis(500);

/// Rough output budget for the session-start digest (~1500 tokens).
const DIGEST_CHAR_BUDGET: usize = 6000;
/// Per-memory content truncation inside the digest.
const DIGEST_ITEM_CHARS: usize = 280;
const DIGEST_MAX_ITEMS: usize = 8;

/// How much of a transcript tail to scan for the last user message.
const TRANSCRIPT_TAIL_BYTES: usize = 64 * 1024;

/// The Claude Code hook events this adapter serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    PreCompact,
    SessionEnd,
}

/// Claude Code hook payload, read from stdin. All fields are optional — a
/// missing or malformed payload must degrade, never fail the hook.
#[derive(Debug, Default, Deserialize)]
struct HookPayload {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
    #[serde(default)]
    #[expect(dead_code, reason = "kept for payload completeness; not needed yet")]
    hook_event_name: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// The fields of a `Memory` search hit the digest actually renders.
/// Everything else (embeddings, metadata, …) is ignored on purpose.
#[derive(Debug, Deserialize)]
struct MemoryHit {
    content: String,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    /// `surrealdb::types::Datetime` serializes as an ISO-8601 string, but stay
    /// tolerant of any shape so one odd record cannot fail the whole recall.
    #[serde(default)]
    created_at: Option<Value>,
}

/// Run a hook subcommand. Errors propagate to the caller, which reports them
/// on stderr and exits 0 regardless.
pub async fn run_hook(event: HookEvent) -> Result<()> {
    let payload = read_hook_payload();
    match event {
        HookEvent::SessionStart => session_start(&payload).await,
        HookEvent::PreCompact => save_session_state(&payload, "pre-compact", HOOK_TIMEOUT).await,
        HookEvent::SessionEnd => {
            save_session_state(&payload, "session-end", SESSION_END_TIMEOUT).await
        }
    }
}

/// Print one compact statusline: server health plus the durable operation
/// count, e.g. `Ψ ok · 4.4K ops`. Prints nothing when the server is down.
pub async fn statusline() -> Result<()> {
    let base = base_url();
    let client = hook_client(STATUSLINE_TIMEOUT)?;

    let health = async {
        client
            .get(format!("{base}/health"))
            .send()
            .await?
            .error_for_status()
            .context("health status")?;
        Ok::<_, anyhow::Error>(())
    };
    let stats = async {
        let body: Value = client
            .get(format!("{base}/api/v2/operations/stats"))
            .send()
            .await?
            .error_for_status()
            .context("stats status")?
            .json()
            .await?;
        let total = body
            .get("counts")
            .and_then(Value::as_object)
            .map(|counts| counts.values().filter_map(Value::as_u64).sum())
            .unwrap_or(0);
        Ok::<_, anyhow::Error>(total)
    };

    let (health, stats) = tokio::join!(health, stats);
    health.context("server unreachable")?;
    match stats {
        Ok(total) => println!("Ψ ok · {}", format_count(total)),
        Err(_) => println!("Ψ ok"),
    }
    Ok(())
}

/// RECALL: semantic search for project memories, rendered as a compact
/// restoration-context digest on stdout (SessionStart stdout becomes agent
/// context). Prints nothing when there is nothing worth recalling.
async fn session_start(payload: &HookPayload) -> Result<()> {
    let project = project_name(payload.cwd.as_deref());
    let client = hook_client(HOOK_TIMEOUT)?;
    let memories: Vec<MemoryHit> = client
        .post(format!("{}/api/v1/search", base_url()))
        .json(&json!({
            "query": format!("{project} session state handoff decisions open threads conventions"),
            "limit": DIGEST_MAX_ITEMS,
        }))
        .send()
        .await
        .context("search request failed")?
        .error_for_status()
        .context("search returned an error status")?
        .json()
        .await
        .context("search response was not a memory list")?;

    if memories.is_empty() {
        return Ok(());
    }

    let mut out = String::from("## Recalled context (surreal-memory)\n");
    for hit in &memories {
        let date = hit
            .created_at
            .as_ref()
            .and_then(Value::as_str)
            .map(|s| s.chars().take(10).collect::<String>())
            .unwrap_or_else(|| "unknown-date".to_owned());
        let kind = hit.memory_type.as_deref().unwrap_or("memory");
        let agent = hit.agent_id.as_deref().unwrap_or("unscoped");
        let content = truncate(&fold_whitespace(&hit.content), DIGEST_ITEM_CHARS);
        let line = format!("- [{date} · {kind} · {agent}] {content}\n");
        if out.len() + line.len() > DIGEST_CHAR_BUDGET {
            break;
        }
        out.push_str(&line);
    }
    print!("{out}");
    Ok(())
}

/// REMEMBER: store a concise session-state memory through the durable
/// operation ledger. `pre-compact` waits for the acknowledgement (accepted =
/// durably persisted); `session-end` runs on a tighter budget and its caller
/// treats a timeout as a dropped write rather than a hook failure.
async fn save_session_state(payload: &HookPayload, event: &str, timeout: Duration) -> Result<()> {
    let project = project_name(payload.cwd.as_deref());
    let session = payload.session_id.as_deref().unwrap_or("unknown");

    let mut content = format!(
        "Session {event} in project '{project}' (session {session}) at {}Z.",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S")
    );
    if let Some(cwd) = &payload.cwd {
        content.push_str(&format!("\ncwd: {cwd}"));
    }
    if let Some(source) = &payload.source {
        content.push_str(&format!("\nsource: {source}"));
    }
    if let Some(transcript) = &payload.transcript_path {
        content.push_str(&format!("\ntranscript: {transcript}"));
        if let Some(message) = last_user_message(transcript) {
            content.push_str(&format!("\nlast user message: \"{message}\""));
        }
    }

    let memory_payload = json!({
        "content": content,
        "user_id": Value::Null,
        "agent_id": AGENT_ID,
        "session_id": session,
        "categories": ["session-state", format!("project:{project}")],
    });
    let hash = payload_hash(&memory_payload)?;
    let operation_id = format!("hook-{event}-{}-{}", sanitize_id(session), &hash[..12]);

    let client = hook_client(timeout)?;
    client
        .post(format!("{}/api/v2/operations", base_url()))
        .json(&json!({
            "operation_id": operation_id,
            "schema_version": 2,
            "kind": "add_memory",
            "dependencies": [],
            "payload_hash": hash,
            "payload": memory_payload,
        }))
        .send()
        .await
        .context("operation submit failed")?
        .error_for_status()
        .context("operation was not accepted")?;
    Ok(())
}

/// Extract the most recent user message from a Claude Code transcript
/// (JSONL), scanning only the tail so large transcripts stay cheap.
fn last_user_message(transcript_path: &str) -> Option<String> {
    let data = std::fs::read(transcript_path).ok()?;
    let start = data.len().saturating_sub(TRANSCRIPT_TAIL_BYTES);
    let tail = String::from_utf8_lossy(&data[start..]);
    for line in tail.lines().rev() {
        let Ok(line) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if line.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let text = match line.get("message").and_then(|m| m.get("content")) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            _ => continue,
        };
        let text = text.trim();
        if !text.is_empty() {
            return Some(truncate(&fold_whitespace(text), 300));
        }
    }
    None
}

fn read_hook_payload() -> HookPayload {
    use std::io::Read;
    let mut buffer = String::new();
    if std::io::stdin().read_to_string(&mut buffer).is_err() {
        return HookPayload::default();
    }
    serde_json::from_str(&buffer).unwrap_or_default()
}

fn base_url() -> String {
    std::env::var("SURREAL_MEMORY_URL")
        .unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned())
        .trim_end_matches('/')
        .to_owned()
}

fn hook_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(1)))
        .build()
        .context("failed to build HTTP client")
}

/// SHA-256 hex of the canonical JSON payload — the exact scheme
/// `src/operations.rs` uses to validate `payload_hash`.
fn payload_hash(payload: &Value) -> Result<String> {
    let canonical = serde_json::to_vec(payload).context("payload serialization failed")?;
    Ok(Sha256::digest(&canonical)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn project_name(cwd: Option<&str>) -> String {
    cwd.and_then(|path| std::path::Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn sanitize_id(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

fn fold_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

fn format_count(count: u64) -> String {
    if count >= 1000 {
        format!("{:.1}K ops", count as f64 / 1000.0)
    } else {
        format!("{count} ops")
    }
}

