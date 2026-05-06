//! Scoped, typed memory system — mem0-compatible.
//!
//! Provides `Memory`, `MemoryScope`, `MemoryType`, and `MemoryHistory` with
//! full audit trail and semantic deduplication support.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use surrealdb::types::{Datetime, RecordId};
use surrealdb_types::SurrealValue;

/// Scoping level for a memory — determines who can see and retrieve it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    /// Visible to all agents and users (system-wide knowledge)
    #[default]
    Global,
    /// Scoped to a specific agent
    Agent,
    /// Scoped to a specific user
    User,
    /// Scoped to a single conversation/session
    Session,
    /// Scoped to a named TaskStream
    Task,
}

impl MemoryScope {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Agent => "agent",
            Self::User => "user",
            Self::Session => "session",
            Self::Task => "task",
        }
    }

    pub fn parse_str(raw: &str) -> Result<Self, String> {
        match raw {
            "global" => Ok(Self::Global),
            "agent" => Ok(Self::Agent),
            "user" => Ok(Self::User),
            "session" => Ok(Self::Session),
            "task" => Ok(Self::Task),
            _ => Err(format!("unknown memory scope '{raw}'")),
        }
    }
}

impl fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_str(s)
    }
}

/// Cognitive memory type — determines storage and retrieval behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Personal events and experiences with temporal context
    Episodic,
    /// General world knowledge, facts, concepts
    #[default]
    Semantic,
    /// Skills, patterns, how-to knowledge
    Procedural,
    /// Connections and associations between concepts
    Associative,
}

impl MemoryType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
            Self::Associative => "associative",
        }
    }

    pub fn parse_str(raw: &str) -> Result<Self, String> {
        match raw {
            "episodic" => Ok(Self::Episodic),
            "semantic" => Ok(Self::Semantic),
            "procedural" => Ok(Self::Procedural),
            "associative" => Ok(Self::Associative),
            _ => Err(format!("unknown memory type '{raw}'")),
        }
    }
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_str(s)
    }
}

/// A single memory unit — the core data structure for the mem0-compatible layer.
/// All optional fields default to `None` for migration safety.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct Memory {
    pub id: Option<RecordId>,
    /// The raw content of the memory.
    pub content: String,
    /// Vector embedding of `content` using the active embedding provider dimensions.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    /// Scoping level — determines who can retrieve this memory.
    #[serde(default)]
    pub scope: MemoryScope,
    /// Cognitive type — influences search ranking and decay strategy.
    #[serde(default)]
    pub memory_type: MemoryType,
    /// Optional user identifier for user-scoped memories.
    pub user_id: Option<String>,
    /// Optional session identifier for session-scoped memories.
    pub session_id: Option<String>,
    /// Optional agent identifier for agent-scoped memories.
    pub agent_id: Option<String>,
    /// Optional TaskStream this memory belongs to.
    pub task_stream_id: Option<RecordId>,
    /// Taxonomy tags — mem0-compatible categories.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Arbitrary JSON metadata.
    pub metadata: Option<serde_json::Value>,
    /// Estimated token count of `content` (used for context budgeting).
    pub token_count: Option<u32>,
    /// Importance score 0.0–1.0. Higher = surfaces first in context windows.
    #[serde(default)]
    pub importance: f32,
    /// Number of times this memory has been accessed.
    #[serde(default)]
    pub access_count: u32,
    /// Last time this memory was returned in a search result.
    pub last_accessed_at: Option<Datetime>,
    /// Optional TTL — memory is considered expired after this timestamp.
    pub valid_until: Option<Datetime>,
    /// Incremented on every update; used for history tracking.
    #[serde(default)]
    pub version: u32,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

impl Memory {
    pub fn new(
        content: impl Into<String>,
        user_id: Option<String>,
        agent_id: Option<String>,
        session_id: Option<String>,
        categories: Vec<String>,
    ) -> Self {
        let now = surrealdb::types::Datetime::default();
        Self {
            id: None,
            content: content.into(),
            embedding: None,
            scope: MemoryScope::default(),
            memory_type: MemoryType::default(),
            user_id,
            agent_id,
            session_id,
            task_stream_id: None,
            categories,
            metadata: None,
            token_count: None,
            importance: 0.5,
            access_count: 0,
            last_accessed_at: None,
            valid_until: None,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Immutable record of a memory mutation — never deleted.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct MemoryHistory {
    pub id: Option<RecordId>,
    /// The memory this record belongs to.
    pub memory_id: RecordId,
    /// Version number at the time of this change.
    pub version: u32,
    /// Previous content (None on `created`).
    pub old_content: Option<String>,
    /// New content after the change.
    pub new_content: String,
    pub changed_at: Datetime,
    /// One of: `created`, `updated`, `deleted`
    pub change_type: String,
}

#[cfg(test)]
mod tests {
    use super::{MemoryScope, MemoryType};

    #[test]
    fn memory_scope_codec_round_trips() {
        for scope in [
            MemoryScope::Global,
            MemoryScope::Agent,
            MemoryScope::User,
            MemoryScope::Session,
            MemoryScope::Task,
        ] {
            assert_eq!(MemoryScope::parse_str(scope.as_str()).unwrap(), scope);
        }
    }

    #[test]
    fn memory_type_codec_round_trips() {
        for memory_type in [
            MemoryType::Episodic,
            MemoryType::Semantic,
            MemoryType::Procedural,
            MemoryType::Associative,
        ] {
            assert_eq!(
                MemoryType::parse_str(memory_type.as_str()).unwrap(),
                memory_type
            );
        }
    }

    #[test]
    fn memory_codecs_reject_unknown_values() {
        assert!(MemoryScope::parse_str("Global").is_err());
        assert!(MemoryType::parse_str("Semantic").is_err());
        assert!(MemoryScope::parse_str("unknown").is_err());
        assert!(MemoryType::parse_str("unknown").is_err());
    }
}
