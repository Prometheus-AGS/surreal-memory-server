//! Text helpers shared by the summarizer, gate and lineage code.
//!
//! One normalisation and one token estimator, so "equivalent claim" and
//! "before/after tokens" mean the same thing everywhere in the pipeline.

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// Lower-cased, NFC-normalised text with whitespace collapsed. Punctuation is
/// kept, so a quote can be matched against its source verbatim-ish.
pub fn normalize(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    nfc.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`normalize`] with punctuation removed: the equivalence key for claims.
pub fn canonical(text: &str) -> String {
    let kept: String = normalize(text)
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Stable hash of [`canonical`] text; identical copies share a hash.
pub fn content_hash(text: &str) -> String {
    Sha256::digest(canonical(text).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Token estimate used for every before/after figure: `ceil(bytes / 4)`, the
/// same rule `SurrealStorage` applies to `token_count`.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4)
}

/// Split text into sentences on line breaks and on `.`, `?`, `!` followed by
/// whitespace or the end of the text. Empty pieces are dropped.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            current.push(c);
            let ends = matches!(c, '.' | '?' | '!');
            if ends && chars.peek().is_none_or(|next| next.is_whitespace()) {
                push_trimmed(&mut out, &mut current);
            }
        }
        push_trimmed(&mut out, &mut current);
    }
    out
}

fn push_trimmed(out: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    current.clear();
}

/// Alphanumeric words of at least `min_len` characters, canonicalised.
pub fn content_words(text: &str, min_len: usize) -> Vec<String> {
    canonical(text)
        .split_whitespace()
        .filter(|word| word.chars().count() >= min_len)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ignores_case_spacing_and_punctuation() {
        assert_eq!(
            canonical("  Always  use TLS!! "),
            canonical("always use tls")
        );
    }

    #[test]
    fn identical_copies_share_a_content_hash() {
        assert_eq!(content_hash("Use TLS."), content_hash("use  tls"));
        assert_ne!(content_hash("Use TLS."), content_hash("Use mTLS."));
    }

    #[test]
    fn sentences_split_on_terminators_and_lines() {
        let parts = sentences("We decided X. Why not Y?\nSecond line");
        assert_eq!(parts, vec!["We decided X.", "Why not Y?", "Second line"]);
    }

    #[test]
    fn sentences_keep_decimal_points_together() {
        assert_eq!(
            sentences("Use version 1.2.3 now."),
            vec!["Use version 1.2.3 now."]
        );
    }

    #[test]
    fn token_estimate_rounds_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }
}
