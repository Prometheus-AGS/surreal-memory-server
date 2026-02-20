//! Model context profile registry — static token budget table for common LLMs.
//!
//! Used by `get_context_for_task` and the rolling auto-summarization worker to
//! determine how many tokens are available for memory injection.
//!
//! Override the default profile by setting `MODEL_ID` env var to any value in the
//! registry (e.g. `claude-3-5-sonnet`). Unknown model IDs fall back to `"default"`.

/// Token budget profile for a single model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProfile {
    /// Short model identifier (matches accepted values for `model_id` parameters).
    pub model_id: &'static str,
    /// Total context window in tokens.
    pub context_window: u64,
    /// Tokens reserved for the model's response. Budget = context_window - reserved.
    pub reserved_tokens: u64,
}

impl ModelProfile {
    /// The effective tokens available for memory injection.
    #[inline]
    pub fn budget(&self) -> u64 {
        self.context_window.saturating_sub(self.reserved_tokens)
    }

    /// 80% of the effective budget — threshold for triggering auto-summarization.
    #[inline]
    pub fn summarization_threshold(&self) -> u64 {
        self.budget() * 80 / 100
    }
}

/// Built-in profiles. Sorted by `model_id` for binary-search lookup.
pub static MODEL_PROFILES: &[ModelProfile] = &[
    ModelProfile {
        model_id: "claude-3-5-sonnet",
        context_window: 200_000,
        reserved_tokens: 16_000,
    },
    ModelProfile {
        model_id: "claude-3-haiku",
        context_window: 200_000,
        reserved_tokens: 16_000,
    },
    ModelProfile {
        model_id: "claude-3-opus",
        context_window: 200_000,
        reserved_tokens: 16_000,
    },
    ModelProfile {
        model_id: "default",
        context_window: 8_000,
        reserved_tokens: 2_000,
    },
    ModelProfile {
        model_id: "gemini-1.5-flash",
        context_window: 1_000_000,
        reserved_tokens: 32_000,
    },
    ModelProfile {
        model_id: "gemini-1.5-pro",
        context_window: 1_000_000,
        reserved_tokens: 32_000,
    },
    ModelProfile {
        model_id: "gemini-2.0-flash",
        context_window: 1_000_000,
        reserved_tokens: 32_000,
    },
    ModelProfile {
        model_id: "gpt-4-turbo",
        context_window: 128_000,
        reserved_tokens: 16_000,
    },
    ModelProfile {
        model_id: "gpt-4o",
        context_window: 128_000,
        reserved_tokens: 16_000,
    },
    ModelProfile {
        model_id: "gpt-4o-mini",
        context_window: 128_000,
        reserved_tokens: 16_000,
    },
    ModelProfile {
        model_id: "llama-3.3-70b",
        context_window: 128_000,
        reserved_tokens: 16_000,
    },
    ModelProfile {
        model_id: "mistral-large",
        context_window: 32_000,
        reserved_tokens: 4_000,
    },
];

/// Look up a profile by `model_id`, falling back to `"default"` if unknown.
///
/// ```rust
/// use surreal_memory::model_profiles::profile_for;
/// let p = profile_for("gpt-4o");
/// assert_eq!(p.budget(), 112_000);
/// ```
pub fn profile_for(model_id: &str) -> &'static ModelProfile {
    MODEL_PROFILES
        .iter()
        .find(|p| p.model_id == model_id)
        .or_else(|| MODEL_PROFILES.iter().find(|p| p.model_id == "default"))
        .expect("default profile always present")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_has_correct_budget() {
        let p = profile_for("gpt-4o");
        assert_eq!(p.budget(), 112_000);
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let p = profile_for("some-unknown-model");
        assert_eq!(p.model_id, "default");
    }

    #[test]
    fn summarization_threshold_is_eighty_percent() {
        let p = profile_for("claude-3-5-sonnet");
        assert_eq!(p.summarization_threshold(), (184_000u64 * 80) / 100);
    }
}
