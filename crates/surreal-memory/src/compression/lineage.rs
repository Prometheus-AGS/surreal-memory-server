//! Lineage: which original evidence a source stands for.
//!
//! A summary carries the roots of the memories it was built from in its
//! metadata, so a summary of a summary resolves to the same original evidence
//! and can never raise its own corroboration.

use std::collections::{BTreeSet, HashSet};

use serde_json::{Value, json};

use super::contract::SourceMemory;
use super::text::content_hash;

/// Metadata key reserved for compression bookkeeping on summary memories.
pub const METADATA_KEY: &str = "_compression";

/// Distinct original evidence behind `sources`.
///
/// A raw memory is its own root, but raw memories with identical content count
/// once (repeated copies are not independent). A summary contributes the roots
/// it was built from. Paraphrased near-copies are not detected.
pub fn independent_roots(sources: &[&SourceMemory]) -> Vec<String> {
    let mut seen_content: HashSet<String> = HashSet::new();
    let mut roots: BTreeSet<String> = BTreeSet::new();
    for source in sources {
        if source.root_sources.is_empty() {
            if seen_content.insert(content_hash(&source.content)) {
                roots.insert(source.id.clone());
            }
        } else {
            roots.extend(source.root_sources.iter().cloned());
        }
    }
    roots.into_iter().collect()
}

/// Metadata stored on a summary memory so later summaries can resolve it.
pub fn summary_metadata(
    derivation_key: &str,
    method: &str,
    roots: &[String],
    contract_version: u32,
) -> Value {
    json!({
        METADATA_KEY: {
            "derivation": derivation_key,
            "method": method,
            "roots": roots,
            "contract_version": contract_version,
        }
    })
}

/// Roots recorded on a summary memory; empty for a raw memory or when the
/// metadata is absent or malformed (unknown stays unknown).
pub fn roots_from_metadata(metadata: &Option<Value>) -> Vec<String> {
    metadata
        .as_ref()
        .and_then(|meta| meta.get(METADATA_KEY))
        .and_then(|block| block.get("roots"))
        .and_then(Value::as_array)
        .map(|roots| {
            roots
                .iter()
                .filter_map(|root| root.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compression::contract::SourceOrigin;

    fn raw(id: &str, content: &str) -> SourceMemory {
        SourceMemory {
            id: id.to_string(),
            version: 1,
            content: content.to_string(),
            origin: SourceOrigin::User,
            root_sources: Vec::new(),
            observed_at: None,
            valid_until: None,
        }
    }

    #[test]
    fn identical_copies_count_once() {
        let a = raw("a", "Never commit secrets.");
        let b = raw("b", "never   commit secrets");
        let c = raw("c", "Pin the SDK version.");
        assert_eq!(
            independent_roots(&[&a, &b, &c]),
            vec!["a".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn a_summary_resolves_to_the_roots_it_was_built_from() {
        let mut summary = raw("s", "[extractive summary] Never commit secrets.");
        summary.root_sources = vec!["a".to_string(), "c".to_string()];
        let a = raw("a", "Never commit secrets.");
        // The summary and its own source add nothing beyond {a, c}.
        assert_eq!(
            independent_roots(&[&summary, &a]),
            vec!["a".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn metadata_round_trips_and_legacy_stays_unknown() {
        let meta = summary_metadata("k", "extractive", &["a".to_string(), "b".to_string()], 1);
        assert_eq!(
            roots_from_metadata(&Some(meta)),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(roots_from_metadata(&None).is_empty());
        assert!(roots_from_metadata(&Some(json!({"agent": "x"}))).is_empty());
    }
}
