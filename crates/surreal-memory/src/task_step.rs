//! TaskStep — first-class, ordered, status-tracked unit of work within a TaskStream.
//!
//! A `TaskStep` brings durable-execution semantics (explicit journaled steps,
//! idempotency keys, checkpoint/resume) to TaskStreams. Steps are linear and
//! ordinal-based — this is durable *tracking*, not orchestration.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use surrealdb::types::{Datetime, RecordId};
use surrealdb_types::SurrealValue;

/// Lifecycle state of a `TaskStep`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue, Default)]
#[serde(rename_all = "lowercase")]
pub enum TaskStepStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl TaskStepStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse_str(raw: &str) -> Result<Self, String> {
        match raw {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            _ => Err(format!("unknown task step status '{raw}'")),
        }
    }

    /// Whether a step in this status is considered done (no further work).
    ///
    /// Only `Completed` and `Skipped` are terminal-done. A `Failed` or
    /// `Running` step is intentionally NOT done: `get_current_step` returns
    /// it as the current step so the caller can resolve it (retry, skip, or
    /// complete) before advancing — a failed step blocks progress until
    /// explicitly resolved.
    pub const fn is_terminal_done(&self) -> bool {
        matches!(self, Self::Completed | Self::Skipped)
    }
}

impl fmt::Display for TaskStepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskStepStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_str(s)
    }
}

/// An ordered, status-tracked, idempotent unit of work within a `TaskStream`.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct TaskStep {
    pub id: Option<RecordId>,
    /// The owning `TaskStream` record. Set by storage when the step is added.
    pub task_stream_id: Option<RecordId>,
    /// Linear position of this step within the stream (1-based by convention).
    pub ordinal: u32,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub status: TaskStepStatus,
    /// Caller-supplied key making step creation and completion safe to replay.
    pub idempotency_key: String,
    /// Result payload recorded when the step completes.
    #[serde(default)]
    pub result: Option<String>,
    /// Error payload recorded when the step fails.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub started_at: Option<Datetime>,
    #[serde(default)]
    pub completed_at: Option<Datetime>,
    pub created_at: Datetime,
}

impl TaskStep {
    /// Construct a new `pending` step. `task_stream_id` is linked by storage.
    pub fn new(
        ordinal: u32,
        name: impl Into<String>,
        description: Option<String>,
        idempotency_key: impl Into<String>,
    ) -> Self {
        Self {
            id: None,
            task_stream_id: None,
            ordinal,
            name: name.into(),
            description,
            status: TaskStepStatus::Pending,
            idempotency_key: idempotency_key.into(),
            result: None,
            error: None,
            started_at: None,
            completed_at: None,
            created_at: Datetime::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TaskStepStatus;

    #[test]
    fn task_step_status_codec_round_trips() {
        for status in [
            TaskStepStatus::Pending,
            TaskStepStatus::Running,
            TaskStepStatus::Completed,
            TaskStepStatus::Failed,
            TaskStepStatus::Skipped,
        ] {
            assert_eq!(TaskStepStatus::parse_str(status.as_str()).unwrap(), status);
        }
    }

    #[test]
    fn task_step_status_codec_rejects_unknown_values() {
        assert!(TaskStepStatus::parse_str("Pending").is_err());
        assert!(TaskStepStatus::parse_str("unknown").is_err());
    }
}
