use chrono::Utc;

use crate::canonical::{CANONICALIZER_VERSION, event_id_for, hash_hex};
use crate::redaction::{RedactionConfig, redact_text};
use crate::severity::{base_severity, effective_severity};
use crate::types::{
    ConfidenceSignal, EvidenceKind, EvidenceRef, ReasoningEventKind, ReasoningQualityEvent,
    VerificationState,
};

pub fn build_user_correction_event(
    session_id: &str,
    turn_number: u64,
    user_input_excerpt: &str,
    prior_events: &[ReasoningQualityEvent],
    redaction: &RedactionConfig,
    shadow: bool,
) -> Option<ReasoningQualityEvent> {
    let prior = most_likely_corrected_claim(turn_number, prior_events)?;
    let event_kind = ReasoningEventKind::ClaimCorrectedByUser;
    let confidence = ConfidenceSignal::Confident;
    let severity_base = base_severity(event_kind);
    let severity_effective = effective_severity(event_kind, confidence, None).unwrap_or(1.0);
    let redacted_excerpt = redact_text(user_input_excerpt, redaction);
    let correction_evidence = EvidenceRef {
        evidence_id: format!("user-correction:{session_id}:{turn_number}"),
        kind: EvidenceKind::UserGroundTruth,
        entity_key: Some(prior.entity_key.clone()),
        output_hash: Some(hash_hex(user_input_excerpt)),
        redacted_excerpt: Some(redacted_excerpt.clone()),
        created_at: Utc::now(),
    };

    Some(ReasoningQualityEvent {
        event_id: event_id_for(
            session_id,
            turn_number,
            &prior.claim_id,
            "ClaimCorrectedByUser",
        ),
        session_id: session_id.to_string(),
        turn_number,
        claim_id: prior.claim_id.clone(),
        event_kind,
        subject: prior.subject,
        entity_key: prior.entity_key.clone(),
        canonicalizer_version: CANONICALIZER_VERSION.to_string(),
        canonical_text: prior.canonical_text.clone(),
        confidence_signal: confidence,
        verification_state: VerificationState::CorrectedByUser,
        severity_base,
        severity_effective,
        evidence_refs: vec![correction_evidence],
        redacted_excerpt: Some(redacted_excerpt),
        raw_text_hash: Some(hash_hex(user_input_excerpt)),
        source_system: "reasoning_quality:user_correction".to_string(),
        shadow,
        created_at: Utc::now(),
        ..ReasoningQualityEvent::default()
    })
}

fn most_likely_corrected_claim(
    turn_number: u64,
    prior_events: &[ReasoningQualityEvent],
) -> Option<&ReasoningQualityEvent> {
    prior_events
        .iter()
        .filter(|event| event.turn_number < turn_number)
        .filter(|event| {
            matches!(
                event.event_kind,
                ReasoningEventKind::ClaimBeforeSourceRead
                    | ReasoningEventKind::UnsupportedClaim
                    | ReasoningEventKind::CompletionClaimWithoutEvidence
                    | ReasoningEventKind::TestStatusClaimWithoutCommand
                    | ReasoningEventKind::VerificationNeeded
                    | ReasoningEventKind::ClaimEmitted
            )
        })
        .max_by(|left, right| {
            left.turn_number
                .cmp(&right.turn_number)
                .then_with(|| {
                    left.severity_effective
                        .partial_cmp(&right.severity_effective)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.event_id.cmp(&right.event_id))
        })
}

#[cfg(test)]
mod tests {
    use crate::types::ReasoningSubject;

    use super::*;

    #[test]
    fn correction_links_to_latest_prior_risky_claim() {
        let prior = ReasoningQualityEvent {
            event_id: "rqevt-prior".into(),
            session_id: "s1".into(),
            turn_number: 2,
            claim_id: "rqclm-prior".into(),
            event_kind: ReasoningEventKind::ClaimBeforeSourceRead,
            subject: ReasoningSubject::Codebase,
            entity_key: "src/lib.rs".into(),
            canonical_text: "src lib rs exists".into(),
            severity_effective: 0.7,
            ..ReasoningQualityEvent::default()
        };
        let event = build_user_correction_event(
            "s1",
            3,
            "no, that file does not exist",
            &[prior],
            &RedactionConfig::default(),
            true,
        )
        .unwrap();

        assert_eq!(event.event_kind, ReasoningEventKind::ClaimCorrectedByUser);
        assert_eq!(event.claim_id, "rqclm-prior");
        assert_eq!(event.verification_state, VerificationState::CorrectedByUser);
        assert_eq!(event.evidence_refs[0].kind, EvidenceKind::UserGroundTruth);
    }
}
