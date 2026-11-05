//! The quality gate between a summarizer's output and publication.
//!
//! Everything here is deterministic and runs without a second model:
//! each claim must quote text that really appears in an input source at the
//! version that was read, the claim must say no more (and not the opposite)
//! of what the quotes say, and the rendered summary must fit the token budget
//! and be meaningfully smaller than its inputs. Anything else is rejected or
//! the whole run is deferred, leaving the sources untouched.

use std::collections::HashMap;

use super::contract::{Candidate, ClaimKind, EvidenceRef, SourceMemory, SummaryMethod};
use super::lineage::independent_roots;
use super::promotion::{self, Promotion};
use super::summarizer::{has_validation_cue, qualifiers_of};
use super::text::{canonical, content_words, estimate_tokens, normalize};

/// Fraction of a claim's content words that must appear in its quotes.
const MIN_SUPPORT: f64 = 0.8;
/// Shortest quote that can support a claim.
const MIN_QUOTE_CHARS: usize = 8;
/// Words shorter than this are ignored by the support check.
const MIN_WORD_LEN: usize = 4;

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    /// Required shrink, e.g. `0.2` means the output must be at most 80% of the input.
    pub min_reduction: f64,
    pub max_candidates: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_tokens: 32_000,
            max_output_tokens: 1_500,
            min_reduction: 0.2,
            max_candidates: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    NoEvidence,
    UnknownSource,
    StaleSourceVersion,
    QuoteNotInSource,
    Unsupported,
    NegationChanged,
    TooManyCandidates,
    DroppedForBudget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub text: String,
    pub reason: RejectReason,
}

#[derive(Clone, Debug)]
pub struct ValidatedClaim {
    pub candidate: Candidate,
    /// Distinct original evidence behind the claim.
    pub roots: Vec<String>,
    pub promotion: Promotion,
}

#[derive(Clone, Debug)]
pub struct GatedSummary {
    pub claims: Vec<ValidatedClaim>,
    pub rejected: Vec<Rejected>,
    pub rendered: String,
    pub tokens_before: u64,
    pub tokens_after: u64,
}

/// Why no summary was produced. The sources and active context stay intact.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeferReason {
    #[error("input is {tokens} tokens, over the limit of {max}")]
    InputTooLarge { tokens: u64, max: u64 },
    #[error("no claim survived validation")]
    NoClaims,
    #[error("protected claims need {tokens} tokens, over the budget of {max}")]
    OverBudget { tokens: u64, max: u64 },
    #[error("output of {after} tokens is not enough smaller than the input of {before}")]
    InsufficientReduction { before: u64, after: u64 },
}

/// Validate `candidates` against `sources` and render the surviving claims.
pub fn run(
    sources: &[SourceMemory],
    candidates: Vec<Candidate>,
    limits: &Limits,
    method: SummaryMethod,
) -> Result<GatedSummary, DeferReason> {
    let tokens_before: u64 = sources.iter().map(|s| estimate_tokens(&s.content)).sum();
    if tokens_before > limits.max_input_tokens {
        return Err(DeferReason::InputTooLarge {
            tokens: tokens_before,
            max: limits.max_input_tokens,
        });
    }

    let by_id: HashMap<&str, &SourceMemory> = sources.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut rejected = Vec::new();
    let mut accepted: Vec<Candidate> = Vec::new();

    for (index, candidate) in candidates.into_iter().enumerate() {
        if index >= limits.max_candidates {
            rejected.push(Rejected {
                text: candidate.text,
                reason: RejectReason::TooManyCandidates,
            });
            continue;
        }
        match validate(candidate, &by_id) {
            Ok(valid) => merge_equivalent(&mut accepted, valid),
            Err(reject) => rejected.push(reject),
        }
    }

    let mut claims: Vec<ValidatedClaim> = accepted
        .into_iter()
        .map(|candidate| {
            let evidence_sources: Vec<&SourceMemory> = candidate
                .evidence
                .iter()
                .filter_map(|e| by_id.get(e.source_id.as_str()).copied())
                .collect();
            let roots = independent_roots(&evidence_sources);
            let origins: Vec<_> = evidence_sources.iter().map(|s| s.origin).collect();
            let promotion = promotion::decide(&candidate, roots.len(), &origins);
            ValidatedClaim {
                candidate,
                roots,
                promotion,
            }
        })
        .collect();

    let mut rendered = render(&claims, method, sources.len());
    while estimate_tokens(&rendered) > limits.max_output_tokens {
        let Some(position) = lowest_priority_droppable(&claims) else {
            return Err(DeferReason::OverBudget {
                tokens: estimate_tokens(&rendered),
                max: limits.max_output_tokens,
            });
        };
        let dropped = claims.remove(position);
        rejected.push(Rejected {
            text: dropped.candidate.text,
            reason: RejectReason::DroppedForBudget,
        });
        rendered = render(&claims, method, sources.len());
    }
    if claims.is_empty() {
        return Err(DeferReason::NoClaims);
    }

    let tokens_after = estimate_tokens(&rendered);
    let allowed = (tokens_before as f64 * (1.0 - limits.min_reduction)).floor() as u64;
    if tokens_after > allowed {
        return Err(DeferReason::InsufficientReduction {
            before: tokens_before,
            after: tokens_after,
        });
    }
    Ok(GatedSummary {
        claims,
        rejected,
        rendered,
        tokens_before,
        tokens_after,
    })
}

fn validate(
    mut candidate: Candidate,
    by_id: &HashMap<&str, &SourceMemory>,
) -> Result<Candidate, Rejected> {
    let reject = |candidate: &Candidate, reason| Rejected {
        text: candidate.text.clone(),
        reason,
    };
    if candidate.evidence.is_empty() {
        return Err(reject(&candidate, RejectReason::NoEvidence));
    }
    for evidence in &candidate.evidence {
        let Some(source) = by_id.get(evidence.source_id.as_str()) else {
            return Err(reject(&candidate, RejectReason::UnknownSource));
        };
        if source.version != evidence.source_version {
            return Err(reject(&candidate, RejectReason::StaleSourceVersion));
        }
        let quote = normalize(&evidence.quote);
        if quote.chars().count() < MIN_QUOTE_CHARS || !normalize(&source.content).contains(&quote) {
            return Err(reject(&candidate, RejectReason::QuoteNotInSource));
        }
    }

    let quotes = candidate
        .evidence
        .iter()
        .map(|e: &EvidenceRef| e.quote.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if !is_supported(&candidate.text, &quotes) {
        return Err(reject(&candidate, RejectReason::Unsupported));
    }
    let claim_wording = qualifiers_of(&candidate.text);
    let quote_wording = qualifiers_of(&quotes);
    if claim_wording.negated != quote_wording.negated {
        return Err(reject(&candidate, RejectReason::NegationChanged));
    }

    // A provider cannot hide a hedge or claim validation the quotes do not make.
    candidate.qualifiers.negated |= claim_wording.negated | quote_wording.negated;
    candidate.qualifiers.uncertain |= claim_wording.uncertain | quote_wording.uncertain;
    candidate.qualifiers.failed_approach |=
        claim_wording.failed_approach | quote_wording.failed_approach;
    candidate.qualifiers.quoted_instruction |=
        claim_wording.quoted_instruction | quote_wording.quoted_instruction;
    candidate.qualifiers.one_off |= claim_wording.one_off | quote_wording.one_off;
    candidate.validated = candidate.validated && has_validation_cue(&quotes);
    Ok(candidate)
}

/// A claim is supported when nearly all of its content words, and every
/// number, appear in the text it quotes.
fn is_supported(claim: &str, quotes: &str) -> bool {
    let quoted = content_words(quotes, 1);
    let has = |word: &String| quoted.iter().any(|q| q == word);
    let numbers_ok = canonical(claim)
        .split_whitespace()
        .filter(|w| w.chars().all(|c| c.is_ascii_digit()))
        .all(|w| quoted.iter().any(|q| q == w));
    let words = content_words(claim, MIN_WORD_LEN);
    if words.is_empty() {
        return numbers_ok;
    }
    let found = words.iter().filter(|w| has(w)).count();
    numbers_ok && (found as f64 / words.len() as f64) >= MIN_SUPPORT
}

fn merge_equivalent(accepted: &mut Vec<Candidate>, incoming: Candidate) {
    let key = canonical(&incoming.text);
    match accepted
        .iter_mut()
        .find(|c| canonical(&c.text) == key && c.kind == incoming.kind)
    {
        Some(existing) => {
            for evidence in incoming.evidence {
                if !existing
                    .evidence
                    .iter()
                    .any(|e| e.source_id == evidence.source_id)
                {
                    existing.evidence.push(evidence);
                }
            }
            existing.validated |= incoming.validated;
        }
        None => accepted.push(incoming),
    }
}

fn lowest_priority_droppable(claims: &[ValidatedClaim]) -> Option<usize> {
    claims
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.candidate.kind.is_protected())
        .min_by_key(|(index, c)| (c.candidate.kind.drop_priority(), std::cmp::Reverse(*index)))
        .map(|(index, _)| index)
}

/// Deterministic rendering of validated claims: only claim text, grouped by
/// kind, so no free text from a provider reaches the stored summary.
fn render(claims: &[ValidatedClaim], method: SummaryMethod, source_count: usize) -> String {
    let sections = [
        (ClaimKind::Constraint, "Constraints"),
        (ClaimKind::Decision, "Decisions"),
        (ClaimKind::Lesson, "Lessons"),
        (ClaimKind::OpenQuestion, "Open questions"),
        (ClaimKind::Conflict, "Conflicts"),
        (ClaimKind::Fact, "Unconfirmed notes"),
    ];
    let mut out = format!("[{} summary of {} memories]", method.as_str(), source_count);
    for (kind, title) in sections {
        let lines: Vec<&str> = claims
            .iter()
            .filter(|c| c.candidate.kind == kind)
            .map(|c| c.candidate.text.as_str())
            .collect();
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{title}:"));
        for line in lines {
            out.push_str(&format!("\n- {line}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compression::contract::{Applicability, Qualifiers, SourceOrigin};
    use crate::compression::summarizer::{ExtractiveSummarizer, Summarizer};

    fn source(id: &str, content: &str) -> SourceMemory {
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

    fn claim(kind: ClaimKind, text: &str, source_id: &str, quote: &str) -> Candidate {
        Candidate {
            kind,
            text: text.to_string(),
            qualifiers: Qualifiers::default(),
            evidence: vec![EvidenceRef {
                source_id: source_id.to_string(),
                source_version: 1,
                quote: quote.to_string(),
            }],
            applicability: Applicability::default(),
            validated: false,
        }
    }

    /// `sentence` inside bulk filler, so a summary can be smaller than its input.
    /// `variant` makes the filler differ, i.e. distinct (not copied) sources.
    fn bulky(sentence: &str, variant: usize) -> String {
        let filler =
            "The team reviewed the calendar, the lunch order and the room bookings in detail. ";
        format!(
            "{filler}{filler}{sentence} {filler}{}",
            "Extra remark. ".repeat(variant)
        )
    }

    fn loose() -> Limits {
        Limits {
            min_reduction: 0.0,
            ..Limits::default()
        }
    }

    #[test]
    fn a_claim_quoting_text_that_is_not_in_the_source_is_rejected() {
        let sources = [source("a", "Never commit secrets to the repository.")];
        let bad = claim(
            ClaimKind::Constraint,
            "Always rotate keys weekly.",
            "a",
            "Always rotate keys weekly.",
        );
        let out = run(&sources, vec![bad], &loose(), SummaryMethod::Semantic);
        assert_eq!(out.unwrap_err(), DeferReason::NoClaims);
    }

    #[test]
    fn a_claim_that_adds_unquoted_content_is_unsupported() {
        let sources = [source("a", "Never commit secrets to the repository.")];
        let inflated = claim(
            ClaimKind::Constraint,
            "Never commit secrets or customer exports to any repository ever.",
            "a",
            "Never commit secrets to the repository.",
        );
        let out = run(&sources, vec![inflated], &loose(), SummaryMethod::Semantic);
        assert_eq!(out.unwrap_err(), DeferReason::NoClaims);
    }

    #[test]
    fn dropping_a_negation_is_rejected() {
        let sources = [source("a", "We must not store raw API keys in logs.")];
        let flipped = claim(
            ClaimKind::Constraint,
            "We must store raw API keys in logs.",
            "a",
            "We must not store raw API keys in logs.",
        );
        assert!(run(&sources, vec![flipped], &loose(), SummaryMethod::Semantic).is_err());
    }

    #[test]
    fn a_stale_source_version_is_rejected() {
        let sources = [source("a", "Never commit secrets to the repository.")];
        let mut stale = claim(
            ClaimKind::Constraint,
            "Never commit secrets.",
            "a",
            "Never commit secrets",
        );
        stale.evidence[0].source_version = 7;
        assert!(run(&sources, vec![stale], &loose(), SummaryMethod::Semantic).is_err());
    }

    #[test]
    fn a_provider_cannot_claim_validation_the_quotes_do_not_make() {
        let sources = [
            source("a", &bulky("Use TLS for all internal traffic.", 1)),
            source("b", &bulky("Use TLS for all internal traffic.", 2)),
        ];
        let mut c = claim(
            ClaimKind::Constraint,
            "Use TLS for all internal traffic.",
            "a",
            "Use TLS for all internal traffic.",
        );
        c.validated = true;
        let out = run(&sources, vec![c], &loose(), SummaryMethod::Semantic).unwrap();
        assert!(!out.claims[0].candidate.validated);
    }

    #[test]
    fn identical_copies_do_not_corroborate_each_other() {
        // Same text modulo case: copies, not independent witnesses.
        let sources = [
            source("a", &bulky("Never commit secrets to the repository.", 0)),
            source("b", &bulky("NEVER commit secrets to the repository.", 0)),
        ];
        let c = claim(
            ClaimKind::Constraint,
            "Never commit secrets to the repository.",
            "a",
            "Never commit secrets to the repository.",
        );
        let mut c2 = c.clone();
        c2.evidence[0].source_id = "b".to_string();
        let out = run(&sources, vec![c, c2], &loose(), SummaryMethod::Semantic).unwrap();
        assert_eq!(out.claims.len(), 1);
        assert_eq!(out.claims[0].roots.len(), 1);
        assert_ne!(
            out.claims[0].promotion.status,
            crate::compression::contract::ClaimStatus::Promoted
        );
    }

    #[tokio::test]
    async fn a_repetitive_stream_shrinks_and_keeps_every_rule_and_question() {
        let noise = "The standup ran long and we talked about lunch plans for the week ahead. ";
        let mut sources = Vec::new();
        for i in 0..6 {
            sources.push(source(
                &format!("m{i}"),
                &format!(
                    "{noise}Never commit secrets to the repository. We decided to use SurrealKV for embedded mode. \
                     Which region do we ship to? {noise}{noise}"
                ),
            ));
        }
        let candidates = ExtractiveSummarizer.extract(&sources).await.unwrap();
        let out = run(
            &sources,
            candidates,
            &Limits::default(),
            SummaryMethod::Extractive,
        )
        .unwrap();
        assert!(out.tokens_after < out.tokens_before);
        let kinds: Vec<_> = out.claims.iter().map(|c| c.candidate.kind).collect();
        assert!(kinds.contains(&ClaimKind::Constraint));
        assert!(kinds.contains(&ClaimKind::Decision));
        assert!(kinds.contains(&ClaimKind::OpenQuestion));
        assert!(
            out.rendered
                .starts_with("[extractive summary of 6 memories]")
        );
        assert!(!out.rendered.contains("lunch"));
    }

    #[test]
    fn output_that_is_not_smaller_is_deferred() {
        let sources = [source("a", "Never commit secrets.")];
        let c = claim(
            ClaimKind::Constraint,
            "Never commit secrets.",
            "a",
            "Never commit secrets.",
        );
        let out = run(
            &sources,
            vec![c],
            &Limits::default(),
            SummaryMethod::Extractive,
        );
        assert!(matches!(
            out,
            Err(DeferReason::InsufficientReduction { .. })
        ));
    }

    #[test]
    fn over_budget_drops_notes_first_and_never_a_constraint() {
        let long = "Never ".to_string()
            + &"deploy on friday evenings because rollback staff are offline. ".repeat(8);
        let sources = [source(
            "a",
            &format!("{long} Maybe the cache is stale and needs a flush soon."),
        )];
        let constraint = claim(ClaimKind::Constraint, long.trim(), "a", long.trim());
        let mut note = claim(
            ClaimKind::Fact,
            "Maybe the cache is stale and needs a flush soon.",
            "a",
            "Maybe the cache is stale and needs a flush soon.",
        );
        note.qualifiers.uncertain = true;
        let tight = Limits {
            max_output_tokens: 140,
            min_reduction: 0.0,
            ..Limits::default()
        };
        let out = run(
            &sources,
            vec![constraint, note],
            &tight,
            SummaryMethod::Extractive,
        )
        .unwrap();
        assert_eq!(out.claims.len(), 1);
        assert_eq!(out.claims[0].candidate.kind, ClaimKind::Constraint);
        assert!(
            out.rejected
                .iter()
                .any(|r| r.reason == RejectReason::DroppedForBudget)
        );
    }
}
