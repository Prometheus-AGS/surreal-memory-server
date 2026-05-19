//! MCP `notifications/progress` heartbeats for slow tool calls.
//!
//! Slow tools (Palace ingest, mindmap updates, `auto_summarize_task_stream`,
//! `compress_memories`) can run for several seconds. Without progress signals
//! an MCP client treats a slow-but-working call as a dead server — exactly how
//! the original 4-minute hang presented.
//!
//! Per the MCP spec (2025-06-18), a server emits `notifications/progress`
//! carrying the client-supplied `progressToken`; clients with
//! `resetTimeoutOnProgress: true` reset their timeout on each heartbeat.
//!
//! [`run_with_progress`] wraps a slow future: while it runs, a background
//! ticker emits a heartbeat every [`HEARTBEAT_INTERVAL`]. When the client
//! supplied no `progressToken`, it degrades to running the future directly
//! with zero overhead.

use std::time::Duration;

use rmcp::{RoleServer, model::ProgressNotificationParam, service::RequestContext};

/// How often to emit a heartbeat while a slow operation runs. Kept well under
/// typical MCP client timeouts so `resetTimeoutOnProgress` keeps the call alive.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Run `future` while emitting MCP progress heartbeats.
///
/// If the request carried a `progressToken`, a background task sends a
/// `notifications/progress` heartbeat every [`HEARTBEAT_INTERVAL`] until the
/// future completes. If there is no token, the future runs unchanged — no task
/// is spawned and there is no overhead (graceful degradation).
///
/// `label` is included in the heartbeat message so a client/operator can see
/// which operation is in flight.
pub async fn run_with_progress<F, T>(ctx: &RequestContext<RoleServer>, label: &str, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let Some(token) = ctx.meta.get_progress_token() else {
        // No progressToken supplied — run the operation directly.
        return future.await;
    };

    let peer = ctx.peer.clone();
    let label = label.to_string();

    // Spawn the heartbeat ticker. It is aborted as soon as the operation
    // future completes — or earlier, if the future panics (see TickerGuard).
    let ticker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        // Skip the immediate first tick so the first heartbeat is a real delay.
        interval.tick().await;
        let mut step: f64 = 0.0;
        loop {
            interval.tick().await;
            step += 1.0;
            let param = ProgressNotificationParam {
                progress_token: token.clone(),
                progress: step,
                total: None,
                message: Some(format!("{label} in progress")),
            };
            if peer.notify_progress(param).await.is_err() {
                // Client disconnected or transport closed — stop heartbeating.
                break;
            }
        }
    });

    // Abort the ticker on ANY exit path from this function — normal completion
    // or an unwinding panic from `future.await`. A bare `JoinHandle` would only
    // be *detached* (not aborted) on panic-unwind, leaking the heartbeat task.
    struct TickerGuard(tokio::task::JoinHandle<()>);
    impl Drop for TickerGuard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _guard = TickerGuard(ticker);

    future.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_interval_is_under_typical_client_timeout() {
        // MCP clients commonly time out around 30-60s. The heartbeat must be
        // comfortably shorter so resetTimeoutOnProgress keeps the call alive.
        assert!(HEARTBEAT_INTERVAL.as_secs() > 0);
        assert!(HEARTBEAT_INTERVAL.as_secs() <= 10);
    }
}
