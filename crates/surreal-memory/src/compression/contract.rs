//! The versioned claim/evidence contract.
//!
//! Extraction produces [`Candidate`]s; promotion is a separate decision made by
//! `promotion::decide`. Stored content is evidence to evaluate, never an
//! instruction or an authority.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Bumped whenever the stored shape or the meaning of a status changes.
pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    Decision,
    Constraint,
    Lesson,
    OpenQuestion,
    Conflict,
    Fact,
}

impl ClaimKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Constraint => "constraint",
            Self::Lesson => "lesson",
            Self::OpenQuestion => "open_question",
            Self::Conflict => "conflict",
            Self::Fact => "fact",
        }
    }

    /// Kinds that may become reusable guidance once promoted.
    pub const fn is_guidance(self) -> bool {
        matches!(self, Self::Decision | Self::Constraint | Self::Lesson)
    }

    /// Kinds that must never be dropped to meet a token budget: dropping one
    /// would silently remove a rule, a pending question or a disagreement.
    pub const fn is_protected(self) -> bool {
        matches!(self, Self::Constraint | Self::OpenQuestion | Self::Conflict)
    }

    /// Lower values are dropped first when the output is over budget.
    pub const fn drop_priority(self) -> u8 {
        match self {
            Self::Fact => 0,
            Self::Lesson => 1,
            Self::Decision => 2,
            Self::Constraint | Self::OpenQuestion | Self::Conflict => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Candidate,
    Promoted,
    Quarantined,
    Rejected,
    Superseded,
    Demoted,
    Stale,
}

impl ClaimStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Promoted => "promoted",
            Self::Quarantined => "quarantined",
            Self::Rejected => "rejected",
            Self::Superseded => "superseded",
            Self::Demoted => "demoted",
            Self::Stale => "stale",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "candidate" => Self::Candidate,
            "promoted" => Self::Promoted,
            "quarantined" => Self::Quarantined,
            "rejected" => Self::Rejected,
            "superseded" => Self::Superseded,
            "demoted" => Self::Demoted,
            "stale" => Self::Stale,
            _ => return None,
        })
    }

    /// Only promoted guidance is returned by default retrieval.
    pub const fn is_default_eligible(self) -> bool {
        matches!(self, Self::Promoted)
    }
}

/// How a summary was produced. Concatenation is never labelled `Semantic`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryMethod {
    Extractive,
    Semantic,
}

impl SummaryMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Extractive => "extractive",
            Self::Semantic => "semantic",
        }
    }
}

/// Who produced a source. Quoted third-party and tool text is not user intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    User,
    Assistant,
    Tool,
    Summary,
    Unknown,
}

impl SourceOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Summary => "summary",
            Self::Unknown => "unknown",
        }
    }

    /// Infer the origin from a memory's category tags (conversation intake
    /// tags each message with its role). Anything unrecognised stays unknown.
    pub fn from_categories(categories: &[String]) -> Self {
        let has = |tag: &str| categories.iter().any(|c| c.eq_ignore_ascii_case(tag));
        if has("auto_summary") || has("compressed") {
            Self::Summary
        } else if has("user") {
            Self::User
        } else if has("assistant") {
            Self::Assistant
        } else if has("tool") || has("system") {
            Self::Tool
        } else {
            Self::Unknown
        }
    }
}

/// One memory handed to the summarizer, frozen at a specific version.
#[derive(Clone, Debug)]
pub struct SourceMemory {
    pub id: String,
    pub version: u32,
    pub content: String,
    pub origin: SourceOrigin,
    /// Original evidence this source stands for. Empty for a raw memory (it is
    /// its own root); the roots of the sources a summary was built from.
    pub root_sources: Vec<String>,
    pub observed_at: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
}

/// Where a claim is meant to apply. Absent fields mean "not stated", never
/// "everywhere".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applicability {
    pub project: Option<String>,
    pub task: Option<String>,
    pub environment: Option<String>,
    pub version: Option<String>,
}

/// Wording that decides whether a claim may be promoted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Qualifiers {
    pub negated: bool,
    pub uncertain: bool,
    pub failed_approach: bool,
    pub quoted_instruction: bool,
    pub one_off: bool,
}

/// A claim's link back to the exact text that supports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source_id: String,
    pub source_version: u32,
    pub quote: String,
}

/// A claim proposed by a summarizer, before the gate and promotion rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub kind: ClaimKind,
    pub text: String,
    #[serde(default)]
    pub qualifiers: Qualifiers,
    pub evidence: Vec<EvidenceRef>,
    #[serde(default)]
    pub applicability: Applicability,
    /// The summarizer's assertion that the claim was explicitly validated. The
    /// gate re-checks it against the quoted text.
    #[serde(default)]
    pub validated: bool,
}

/// How a summarizer identifies itself in the derivation record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummarizerDescriptor {
    pub method: SummaryMethod,
    pub algorithm: String,
    pub model: Option<String>,
    pub prompt_version: Option<String>,
    pub config_version: String,
}

/// A stored claim as returned by the audit read. `last_verified_at` is
/// deliberately absent: compression never verifies anything, so it stays
/// unknown until a reviewer sets it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimRecord {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub status: String,
    pub reason: Option<String>,
    pub evidence_strength: Option<String>,
    pub corroboration: u32,
    pub evidence: Vec<EvidenceRecord>,
}

/// One quoted piece of evidence behind a stored claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub source_id: String,
    pub source_version: u32,
    pub quote: String,
    pub origin: String,
    pub root_source: String,
}
