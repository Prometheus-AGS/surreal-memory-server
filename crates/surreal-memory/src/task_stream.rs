//! TaskStream — named, long-running task context with model-aware token budgeting.
//!
//! A `TaskStream` accumulates memories scoped to a specific workload (e.g., a
//! coding session, a research task). `ContextWindow` provides the token-budget-
//! aware view for injecting into a model prompt.

use serde::{Deserialize, Serialize};
use surrealdb::types::{Datetime, RecordId};
use surrealdb_types::SurrealValue;

use crate::memory::Memory;
use crate::model_profiles::{ModelProfile, profile_for};

/// Lifecycle state of a TaskStream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue, Default)]
#[serde(rename_all = "lowercase")]
pub enum TaskStreamStatus {
    #[default]
    Active,
    Paused,
    Archived,
}

/// A named, long-running task context that accumulates scoped memories.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct TaskStream {
    pub id: Option<RecordId>,
    /// Human-readable name, unique per agent/user.
    pub name: String,
    pub description: Option<String>,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
    #[serde(default)]
    pub status: TaskStreamStatus,
    /// Running total of tokens across all memories in this stream.
    #[serde(default)]
    pub total_tokens: u64,
    /// Model profile ID governing the token budget for auto-summarization.
    /// Falls back to `"default"` if not set.
    pub model_id: Option<String>,
    /// When `true` (default), rolling auto-summarization fires via the TTL worker
    /// whenever `total_tokens` exceeds 80% of the model's context budget.
    #[serde(default = "default_auto_summarize")]
    pub auto_summarize: bool,
    /// How many rolling summarizations have been performed on this stream.
    #[serde(default)]
    pub summary_count: u32,
    pub created_at: Datetime,
    pub last_active: Datetime,
}

fn default_auto_summarize() -> bool {
    true
}

impl TaskStream {
    pub fn new(
        name: impl Into<String>,
        description: Option<String>,
        agent_id: Option<String>,
        user_id: Option<String>,
    ) -> Self {
        let now = surrealdb::types::Datetime::default();
        Self {
            id: None,
            name: name.into(),
            description,
            agent_id,
            user_id,
            status: TaskStreamStatus::Active,
            total_tokens: 0,
            model_id: None,
            auto_summarize: true,
            summary_count: 0,
            created_at: now.clone(),
            last_active: now,
        }
    }

    /// Return the effective model profile for this stream.
    pub fn profile(&self) -> &'static ModelProfile {
        profile_for(self.model_id.as_deref().unwrap_or("default"))
    }

    /// Check whether auto-summarization should trigger now.
    pub fn needs_summarization(&self) -> bool {
        self.auto_summarize && self.total_tokens >= self.profile().summarization_threshold()
    }
}

/// Token-budget-aware context window for injecting task memories into a model prompt.
#[derive(Debug, Serialize, Deserialize)]
pub struct ContextWindow {
    /// Memories included within the budget, ordered by importance desc.
    pub memories: Vec<Memory>,
    /// Total tokens consumed by included memories.
    pub tokens_used: u64,
    /// Number of memories that were excluded due to token budget.
    pub memories_omitted: u64,
    /// The model name used to calculate context budget.
    pub model_name: String,
    /// The effective token budget (model limit minus reserved-for-response).
    pub token_budget: u64,
}
