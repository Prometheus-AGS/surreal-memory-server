//! Background TTL decay worker.
//!
//! Runs on a configurable interval (default: every 60 minutes) and calls
//! `MemoryStorage::expire_stale_memories_report()` to physically delete
//! records whose `valid_until` has passed, keeping a `deleted` history row for
//! each. Expired memories are already hidden from normal reads, so this pass is
//! cleanup, not what makes them invisible.
//!
//! A failed deletion is logged as an error and retried on the next pass; only
//! confirmed deletions are counted as removed.
//!
//! Configure via the `TTL_INTERVAL_SECS` environment variable (must be a
//! positive integer; anything else falls back to the default with a warning).

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
    let interval_secs = interval_from_env(std::env::var("TTL_INTERVAL_SECS").ok().as_deref());

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

/// A positive interval from the raw env value, or the default with a warning.
fn interval_from_env(raw: Option<&str>) -> u64 {
    match raw {
        None => DEFAULT_INTERVAL_SECS,
        Some(value) => match value.trim().parse::<u64>() {
            Ok(secs) if secs > 0 => secs,
            _ => {
                tracing::warn!(
                    value,
                    default = DEFAULT_INTERVAL_SECS,
                    "TTL_INTERVAL_SECS must be a positive integer; using the default"
                );
                DEFAULT_INTERVAL_SECS
            }
        },
    }
}

async fn run_sweep(storage: &Arc<dyn MemoryStorage>) {
    match storage.expire_stale_memories_report().await {
        Ok(report) if report.failed > 0 => {
            tracing::error!(
                selected = report.selected,
                removed = report.removed,
                skipped = report.skipped,
                failed = report.failed,
                "TTL sweep incomplete: failed deletions will be retried on the next pass"
            );
        }
        Ok(report) if report.removed == 0 && report.skipped == 0 => {
            tracing::debug!("TTL sweep: no expired memories found");
        }
        Ok(report) => {
            tracing::info!(
                removed = report.removed,
                skipped = report.skipped,
                "TTL sweep removed expired memories"
            );
        }
        Err(error) => {
            tracing::error!(%error, "TTL sweep failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_accepts_only_positive_integers() {
        assert_eq!(interval_from_env(None), DEFAULT_INTERVAL_SECS);
        assert_eq!(interval_from_env(Some("120")), 120);
        assert_eq!(interval_from_env(Some("0")), DEFAULT_INTERVAL_SECS);
        assert_eq!(interval_from_env(Some("soon")), DEFAULT_INTERVAL_SECS);
        assert_eq!(interval_from_env(Some("-5")), DEFAULT_INTERVAL_SECS);
    }
}
