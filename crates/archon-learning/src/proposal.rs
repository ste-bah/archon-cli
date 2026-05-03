//! BehaviourProposal generation from LearningEvent aggregation.
//!
//! Rule-based aggregation: scans a window of LearningEvents and emits
//! BehaviourProposals when thresholds are met.
//!
//! ## Aggregation rules
//!
//! 1. **SourceContradicted threshold**: >=3 SourceContradicted events for the
//!    same source_artifact_id within 7 days → SourceQualityProfile proposal
//!    lowering that source's weight by 0.15.
//!
//! 2. **GateFailed threshold**: >=3 GateFailed events for the same gate within
//!    7 days → PipelineGates proposal recommending gate adjustment.

use std::collections::HashMap;

use crate::models::*;

/// Generate proposals from a window of learning events.
///
/// Groups events by type and source, applies thresholds, and emits
/// proposals when aggregation criteria are met.
pub fn generate_proposals(events: &[LearningEvent]) -> Vec<BehaviourProposal> {
    let mut proposals = Vec::new();

    proposals.extend(check_source_contradictions(events));
    proposals.extend(check_gate_failures(events));

    proposals
}

/// Rule 1: >=3 SourceContradicted for same source → SourceQualityProfile proposal.
fn check_source_contradictions(events: &[LearningEvent]) -> Vec<BehaviourProposal> {
    let mut proposals = Vec::new();

    // Group SourceContradicted events by source_artifact_id
    let mut source_counts: HashMap<&str, Vec<&LearningEvent>> = HashMap::new();
    for event in events {
        if event.event_type == LearningEventType::SourceContradicted {
            source_counts
                .entry(event.source_artifact_id.as_str())
                .or_default()
                .push(event);
        }
    }

    for (source_id, source_events) in &source_counts {
        if source_events.len() >= 3 {
            let evidence_ids: Vec<String> = source_events
                .iter()
                .map(|e| e.event_id.clone())
                .collect();

            let new_weight = 0.85_f32.powi(source_events.len() as i32).max(0.1);

            let proposal = BehaviourProposal {
                proposal_id: format!(
                    "bp-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
                ),
                workspace_id: source_events[0].workspace_id.clone(),
                manifest_kind: BehaviourManifestKind::SourceQualityProfile,
                current_version: String::new(), // filled by apply step
                proposed_version: format!("v-{}", uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()),
                diff: format!(
                    "--- SourceQualityProfile: {source}\n\
                     +++ SourceQualityProfile: {source}\n\
                     @@ weight adjustment @@\n\
                     -weight: previous\n\
                     +weight: {new_weight:.2}\n\
                     Reason: {count} contradictions in window",
                    source = source_id,
                    count = source_events.len(),
                ),
                evidence_ids,
                risk_level: BehaviourManifestKind::SourceQualityProfile.default_risk_level(),
                policy_decision: PolicyDecision::PendingApproval, // set by policy engine
                status: ProposalStatus::Pending,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            proposals.push(proposal);
        }
    }

    proposals
}

/// Rule 2: >=3 GateFailed for same gate → PipelineGates proposal.
fn check_gate_failures(events: &[LearningEvent]) -> Vec<BehaviourProposal> {
    let mut proposals = Vec::new();

    let mut gate_counts: HashMap<&str, Vec<&LearningEvent>> = HashMap::new();
    for event in events {
        if event.event_type == LearningEventType::GateFailed {
            gate_counts
                .entry(event.source_artifact_id.as_str())
                .or_default()
                .push(event);
        }
    }

    for (gate_name, gate_events) in &gate_counts {
        if gate_events.len() >= 3 {
            let evidence_ids: Vec<String> = gate_events
                .iter()
                .map(|e| e.event_id.clone())
                .collect();

            let proposal = BehaviourProposal {
                proposal_id: format!(
                    "bp-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
                ),
                workspace_id: gate_events[0].workspace_id.clone(),
                manifest_kind: BehaviourManifestKind::PipelineGates,
                current_version: String::new(),
                proposed_version: format!("v-{}", uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()),
                diff: format!(
                    "--- PipelineGates: {gate}\n\
                     +++ PipelineGates: {gate}\n\
                     @@ gate adjustment @@\n\
                     -status: normal\n\
                     +status: elevated-scrutiny\n\
                     Reason: {count} failures in window",
                    gate = gate_name,
                    count = gate_events.len(),
                ),
                evidence_ids,
                risk_level: BehaviourManifestKind::PipelineGates.default_risk_level(),
                policy_decision: PolicyDecision::PendingApproval,
                status: ProposalStatus::Pending,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            proposals.push(proposal);
        }
    }

    proposals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(
        event_type: LearningEventType,
        source_id: &str,
        workspace_id: &str,
    ) -> LearningEvent {
        LearningEvent {
            event_id: format!("ev-{}", uuid::Uuid::new_v4()),
            workspace_id: workspace_id.to_string(),
            event_type,
            source_artifact_id: source_id.to_string(),
            outcome_artifact_id: None,
            signal: serde_json::json!({}),
            confidence: 0.8,
            provenance_record_id: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_three_contradictions_emit_proposal() {
        let events = vec![
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
        ];
        let proposals = generate_proposals(&events);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].manifest_kind, BehaviourManifestKind::SourceQualityProfile);
        assert_eq!(proposals[0].evidence_ids.len(), 3);
    }

    #[test]
    fn test_two_contradictions_emit_nothing() {
        let events = vec![
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
        ];
        let proposals = generate_proposals(&events);
        assert_eq!(proposals.len(), 0);
    }

    #[test]
    fn test_mixed_sources_contradictions() {
        let events = vec![
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
            make_event(LearningEventType::SourceContradicted, "source-1", "ws-1"),
            make_event(LearningEventType::SourceContradicted, "source-2", "ws-1"),
            make_event(LearningEventType::SourceContradicted, "source-2", "ws-1"),
        ];
        let proposals = generate_proposals(&events);
        // Only source-1 has >=3, source-2 has 2
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].evidence_ids.len(), 3);
    }

    #[test]
    fn test_three_gate_failures_emit_proposal() {
        let events = vec![
            make_event(LearningEventType::GateFailed, "gate-sherlock", "ws-1"),
            make_event(LearningEventType::GateFailed, "gate-sherlock", "ws-1"),
            make_event(LearningEventType::GateFailed, "gate-sherlock", "ws-1"),
        ];
        let proposals = generate_proposals(&events);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].manifest_kind, BehaviourManifestKind::PipelineGates);
    }
}
