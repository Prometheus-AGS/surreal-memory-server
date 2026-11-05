//! Promotion: the decision to let an extracted claim act as reusable guidance.
//!
//! Extraction finds candidates; this module decides, with a recorded reason,
//! whether one is promoted, left as a candidate or quarantined. The rules are
//! deliberately conservative and wording-based. They are not a truth oracle:
//! a promoted claim means "enough independent, unhedged evidence", not "true".

use super::contract::{Candidate, ClaimStatus, SourceOrigin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceStrength {
    /// One independent root, not validated.
    Single,
    /// Several independent roots agree.
    Corroborated(usize),
    /// The evidence explicitly states the claim was validated.
    Validated,
}

impl EvidenceStrength {
    pub fn label(self) -> String {
        match self {
            Self::Single => "single".to_string(),
            Self::Corroborated(n) => format!("corroborated:{n}"),
            Self::Validated => "validated".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Promotion {
    pub status: ClaimStatus,
    pub reason: &'static str,
    pub strength: EvidenceStrength,
}

/// Independent roots required to promote a claim that carries no explicit
/// validation statement.
pub const MIN_INDEPENDENT_ROOTS: usize = 2;

/// Decide the status of `candidate` given how many independent original
/// sources support it and who produced them.
pub fn decide(
    candidate: &Candidate,
    independent_roots: usize,
    origins: &[SourceOrigin],
) -> Promotion {
    let strength = if candidate.validated {
        EvidenceStrength::Validated
    } else if independent_roots >= MIN_INDEPENDENT_ROOTS {
        EvidenceStrength::Corroborated(independent_roots)
    } else {
        EvidenceStrength::Single
    };
    let held = |status, reason| Promotion {
        status,
        reason,
        strength,
    };

    if !candidate.kind.is_guidance() {
        return held(ClaimStatus::Candidate, "not_guidance");
    }
    let q = candidate.qualifiers;
    if q.quoted_instruction {
        return held(ClaimStatus::Quarantined, "quoted_instruction");
    }
    if q.uncertain {
        return held(ClaimStatus::Quarantined, "uncertain");
    }
    if q.failed_approach {
        return held(ClaimStatus::Quarantined, "failed_approach");
    }
    if q.one_off {
        return held(ClaimStatus::Quarantined, "one_off_preference");
    }
    let from_a_person_or_agent = origins
        .iter()
        .any(|origin| matches!(origin, SourceOrigin::User | SourceOrigin::Assistant));
    if !from_a_person_or_agent {
        return held(ClaimStatus::Quarantined, "non_user_origin");
    }
    match strength {
        EvidenceStrength::Validated => held(ClaimStatus::Promoted, "validated"),
        EvidenceStrength::Corroborated(_) => held(ClaimStatus::Promoted, "corroborated"),
        EvidenceStrength::Single => {
            held(ClaimStatus::Candidate, "insufficient_independent_evidence")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compression::contract::{Applicability, ClaimKind, Qualifiers};

    fn candidate(kind: ClaimKind, qualifiers: Qualifiers, validated: bool) -> Candidate {
        Candidate {
            kind,
            text: "x".to_string(),
            qualifiers,
            evidence: Vec::new(),
            applicability: Applicability::default(),
            validated,
        }
    }

    const USER: &[SourceOrigin] = &[SourceOrigin::User];

    #[test]
    fn two_independent_roots_promote_a_constraint() {
        let p = decide(
            &candidate(ClaimKind::Constraint, Qualifiers::default(), false),
            2,
            USER,
        );
        assert_eq!(p.status, ClaimStatus::Promoted);
        assert_eq!(p.strength, EvidenceStrength::Corroborated(2));
    }

    #[test]
    fn one_root_is_not_enough_without_validation() {
        let p = decide(
            &candidate(ClaimKind::Decision, Qualifiers::default(), false),
            1,
            USER,
        );
        assert_eq!(p.status, ClaimStatus::Candidate);
        assert_eq!(p.reason, "insufficient_independent_evidence");
    }

    #[test]
    fn explicit_validation_promotes_from_one_root() {
        let p = decide(
            &candidate(ClaimKind::Lesson, Qualifiers::default(), true),
            1,
            USER,
        );
        assert_eq!(p.status, ClaimStatus::Promoted);
        assert_eq!(p.strength, EvidenceStrength::Validated);
    }

    #[test]
    fn hedged_failed_injected_and_one_off_claims_never_promote() {
        for (q, reason) in [
            (
                Qualifiers {
                    uncertain: true,
                    ..Default::default()
                },
                "uncertain",
            ),
            (
                Qualifiers {
                    failed_approach: true,
                    ..Default::default()
                },
                "failed_approach",
            ),
            (
                Qualifiers {
                    quoted_instruction: true,
                    ..Default::default()
                },
                "quoted_instruction",
            ),
            (
                Qualifiers {
                    one_off: true,
                    ..Default::default()
                },
                "one_off_preference",
            ),
        ] {
            // Even with plenty of evidence and a validation statement.
            let p = decide(&candidate(ClaimKind::Constraint, q, true), 5, USER);
            assert_eq!(p.status, ClaimStatus::Quarantined);
            assert_eq!(p.reason, reason);
        }
    }

    #[test]
    fn tool_only_evidence_is_quarantined() {
        let p = decide(
            &candidate(ClaimKind::Constraint, Qualifiers::default(), false),
            3,
            &[SourceOrigin::Tool, SourceOrigin::Unknown],
        );
        assert_eq!(p.status, ClaimStatus::Quarantined);
        assert_eq!(p.reason, "non_user_origin");
    }

    #[test]
    fn questions_and_facts_are_not_guidance() {
        let p = decide(
            &candidate(ClaimKind::OpenQuestion, Qualifiers::default(), true),
            4,
            USER,
        );
        assert_eq!(p.status, ClaimStatus::Candidate);
        assert_eq!(p.reason, "not_guidance");
    }
}
