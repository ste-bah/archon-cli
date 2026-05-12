use crate::canonical::event_id_for;
use crate::severity::{base_severity, effective_severity};
use crate::types::{
    EvidenceRef, ReasoningEventKind, ReasoningQualityEvent, ReasoningSubject, VerificationState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceMatch {
    pub state: VerificationState,
    pub refs: Vec<EvidenceRef>,
}

pub fn match_claim_evidence(
    subject: ReasoningSubject,
    entity_key: &str,
    evidence_refs: &[EvidenceRef],
) -> EvidenceMatch {
    if evidence_refs.is_empty() {
        return EvidenceMatch {
            state: VerificationState::Unverified,
            refs: Vec::new(),
        };
    }

    let mut exact = Vec::new();
    let mut partial = Vec::new();
    for evidence in evidence_refs {
        if let Some(evidence_entity) = &evidence.entity_key {
            if !entity_key.is_empty() && same_entity(evidence_entity, entity_key) {
                exact.push(evidence.clone());
                continue;
            }
            if !entity_key.is_empty()
                && (evidence_entity.contains(entity_key) || entity_key.contains(evidence_entity))
            {
                partial.push(evidence.clone());
            }
        } else if accepts_subject_only(subject) {
            partial.push(evidence.clone());
        }
    }

    if !exact.is_empty() {
        EvidenceMatch {
            state: VerificationState::VerifiedBeforeClaim,
            refs: exact,
        }
    } else if !partial.is_empty() {
        EvidenceMatch {
            state: VerificationState::PartiallyVerified,
            refs: partial,
        }
    } else {
        EvidenceMatch {
            state: VerificationState::NeedsHumanReview,
            refs: Vec::new(),
        }
    }
}

pub fn build_superseding_source_events(
    prior_events: &[ReasoningQualityEvent],
    evidence_refs: &[EvidenceRef],
) -> Vec<ReasoningQualityEvent> {
    let mut out = Vec::new();
    for prior in prior_events {
        if !needs_later_verification(prior) {
            continue;
        }
        let Some(evidence) = evidence_refs
            .iter()
            .find(|evidence| evidence_matches_prior(evidence, prior))
        else {
            continue;
        };
        let contradicted = evidence_contradicts(evidence);
        let event_kind = if contradicted {
            ReasoningEventKind::ClaimContradictedBySource
        } else {
            ReasoningEventKind::SourceVerifiedClaim
        };
        let mut event = prior.clone();
        event.event_kind = event_kind;
        event.verification_state = if contradicted {
            VerificationState::Contradicted
        } else {
            VerificationState::VerifiedAfterClaim
        };
        event.severity_base = base_severity(event_kind);
        event.severity_effective =
            effective_severity(event_kind, event.confidence_signal, None).unwrap_or(0.0);
        event.evidence_refs = vec![evidence.clone()];
        event.source_system = "reasoning_quality_chronology".to_string();
        event.created_at = chrono::Utc::now();
        event.event_id = event_id_for(
            &event.session_id,
            event.turn_number,
            &event.claim_id,
            &format!("superseding:{event_kind:?}:{}", evidence.evidence_id),
        );
        out.push(event);
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn same_entity(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
        || left
            .trim_start_matches("./")
            .eq_ignore_ascii_case(right.trim_start_matches("./"))
}

fn needs_later_verification(event: &ReasoningQualityEvent) -> bool {
    matches!(
        event.event_kind,
        ReasoningEventKind::ClaimBeforeSourceRead
            | ReasoningEventKind::UnsupportedClaim
            | ReasoningEventKind::CompletionClaimWithoutEvidence
            | ReasoningEventKind::TestStatusClaimWithoutCommand
            | ReasoningEventKind::VerificationNeeded
    ) && matches!(
        event.verification_state,
        VerificationState::Unverified
            | VerificationState::NeedsHumanReview
            | VerificationState::PartiallyVerified
    )
}

fn evidence_matches_prior(evidence: &EvidenceRef, prior: &ReasoningQualityEvent) -> bool {
    evidence
        .entity_key
        .as_deref()
        .is_some_and(|entity| same_entity(entity, &prior.entity_key))
}

fn evidence_contradicts(evidence: &EvidenceRef) -> bool {
    let Some(text) = &evidence.redacted_excerpt else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    lower.contains("no such file")
        || lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("failed to read")
}

fn accepts_subject_only(subject: ReasoningSubject) -> bool {
    matches!(
        subject,
        ReasoningSubject::ExternalFact
            | ReasoningSubject::ArchitectureAdvice
            | ReasoningSubject::GeneralReasoning
    )
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::types::EvidenceKind;

    use super::*;

    #[test]
    fn exact_entity_is_verified_before_claim() {
        let evidence = EvidenceRef {
            evidence_id: "ev1".to_string(),
            kind: EvidenceKind::FileRead,
            entity_key: Some("src/lib.rs".to_string()),
            created_at: Utc::now(),
            ..EvidenceRef::default()
        };
        let matched = match_claim_evidence(ReasoningSubject::Codebase, "./src/lib.rs", &[evidence]);
        assert_eq!(matched.state, VerificationState::VerifiedBeforeClaim);
    }

    #[test]
    fn later_missing_file_output_supersedes_prior_claim() {
        let prior = ReasoningQualityEvent {
            session_id: "s1".into(),
            turn_number: 1,
            claim_id: "rqclm_a".into(),
            event_kind: ReasoningEventKind::ClaimBeforeSourceRead,
            verification_state: VerificationState::Unverified,
            subject: ReasoningSubject::Codebase,
            entity_key: "src/missing.rs".into(),
            ..ReasoningQualityEvent::default()
        };
        let evidence = EvidenceRef {
            evidence_id: "ev2".into(),
            entity_key: Some("src/missing.rs".into()),
            redacted_excerpt: Some("No such file or directory".into()),
            created_at: Utc::now(),
            ..EvidenceRef::default()
        };
        let events = build_superseding_source_events(&[prior], &[evidence]);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].event_kind,
            ReasoningEventKind::ClaimContradictedBySource
        );
    }
}
