//! Background TTL decay worker.
//!
//! Runs on a configurable interval (default: every 60 minutes) and calls
//! `MemoryStorage::expire_stale_memories()` to soft-delete records whose
//! `valid_until` timestamp has elapsed.
//!
//! Configure via `TTL_INTERVAL_SECS` environment variable.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::storage::MemoryStorage;

/// Default interval between TTL sweep passes (60 minutes).
const DEFAULT_INTERVAL_SECS: u64 = 3600;

/// Run the background TTL sweep indefinitely until the provided shutdown signal fires.
pub async fn run_ttl_worker(
    storage: Arc<dyn MemoryStorage>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let interval_secs = std::env::var("TTL_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS);

    let interval = Duration::from_secs(interval_secs);
    tracing::info!("⏰ TTL decay worker started (interval: {}s)", interval_secs);

    // Run an initial pass immediately on startup.
    run_sweep(&storage).await;

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                run_sweep(&storage).await;
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("⏰ TTL decay worker shutting down");
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn run_sweep(storage: &Arc<dyn MemoryStorage>) {
    match storage.expire_stale_memories().await {
        Ok(0) => {
            tracing::debug!("TTL sweep: no stale memories found");
        }
        Ok(n) => {
            tracing::info!("TTL sweep: expired {} stale memories", n);
        }
        Err(e) => {
            tracing::error!("TTL sweep error: {}", e);
        }
    }
}
