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
//!
//! 3. **UserCorrected cluster**: >=3 UserCorrected events for the same
//!    source_artifact_id (top rule id) within 7 days →
//!    BehaviouralRuleAdjustment proposal recommending operator review.

use std::collections::HashMap;

use crate::models::*;

const PROPOSAL_WINDOW_DAYS: i64 = 7;

/// Generate proposals from a window of learning events.
///
/// Groups events by type and source, applies thresholds, and emits
/// proposals when aggregation criteria are met.
pub fn generate_proposals(events: &[LearningEvent]) -> Vec<BehaviourProposal> {
    let mut proposals = Vec::new();

    proposals.extend(check_source_contradictions(events));
    proposals.extend(check_gate_failures(events));
    proposals.extend(check_user_correction_clusters(events));

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
            let evidence_ids: Vec<String> =
                source_events.iter().map(|e| e.event_id.clone()).collect();

            let new_weight = 0.85_f32.powi(source_events.len() as i32).max(0.1);

            let proposal = BehaviourProposal {
                proposal_id: format!(
                    "bp-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
                ),
                workspace_id: source_events[0].workspace_id.clone(),
                manifest_kind: BehaviourManifestKind::SourceQualityProfile,
                current_version: String::new(), // filled by apply step
                proposed_version: format!(
                    "v-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
                ),
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
            let evidence_ids: Vec<String> =
                gate_events.iter().map(|e| e.event_id.clone()).collect();

            let proposal = BehaviourProposal {
                proposal_id: format!(
                    "bp-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
                ),
                workspace_id: gate_events[0].workspace_id.clone(),
                manifest_kind: BehaviourManifestKind::PipelineGates,
                current_version: String::new(),
                proposed_version: format!(
                    "v-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
                ),
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

/// Rule 3: >=3 UserCorrected for same rule id within 7 days.
fn check_user_correction_clusters(events: &[LearningEvent]) -> Vec<BehaviourProposal> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(PROPOSAL_WINDOW_DAYS);
    let mut rule_counts: HashMap<&str, Vec<&LearningEvent>> = HashMap::new();

    for event in events {
        if event.event_type != LearningEventType::UserCorrected {
            continue;
        }
        if event.source_artifact_id.is_empty() {
            continue;
        }
        if !is_within_window(event, cutoff) {
            continue;
        }
        rule_counts
            .entry(event.source_artifact_id.as_str())
            .or_default()
            .push(event);
    }

    rule_counts
        .into_iter()
        .filter(|(_, rule_events)| rule_events.len() >= 3)
        .map(|(rule_id, rule_events)| {
            let evidence_ids: Vec<String> =
                rule_events.iter().map(|e| e.event_id.clone()).collect();
            let correction_count = rule_events.len();
            BehaviourProposal {
                proposal_id: format!(
                    "bp-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
                ),
                workspace_id: rule_events[0].workspace_id.clone(),
                manifest_kind: BehaviourManifestKind::BehaviouralRuleAdjustment,
                current_version: String::new(),
                proposed_version: format!(
                    "v-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
                ),
                diff: format!(
                    "--- BehaviouralRule: {rule_id}\n\
                     +++ BehaviouralRule: {rule_id}\n\
                     @@ operator review @@\n\
                     payload_json: {payload}\n\
                     Reason: {correction_count} user corrections in {window_days} days clustered on rule {rule_id}",
                    payload = serde_json::json!({
                        "rule_id": rule_id,
                        "correction_count": correction_count,
                        "window_days": PROPOSAL_WINDOW_DAYS,
                    }),
                    window_days = PROPOSAL_WINDOW_DAYS,
                ),
                evidence_ids,
                risk_level: BehaviourManifestKind::BehaviouralRuleAdjustment.default_risk_level(),
                policy_decision: PolicyDecision::PendingApproval,
                status: ProposalStatus::Pending,
                created_at: chrono::Utc::now().to_rfc3339(),
            }
        })
        .collect()
}

fn is_within_window(event: &LearningEvent, cutoff: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(&event.created_at)
        .map(|ts| ts.with_timezone(&chrono::Utc) >= cutoff)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(
        event_type: LearningEventType,
        source_id: &str,
        workspace_id: &str,
    ) -> LearningEvent {
        make_event_at(
            event_type,
            source_id,
            workspace_id,
            chrono::Utc::now().to_rfc3339(),
        )
    }

    fn make_event_at(
        event_type: LearningEventType,
        source_id: &str,
        workspace_id: &str,
        created_at: String,
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
            created_at,
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
        assert_eq!(
            proposals[0].manifest_kind,
            BehaviourManifestKind::SourceQualityProfile
        );
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
        assert_eq!(
            proposals[0].manifest_kind,
            BehaviourManifestKind::PipelineGates
        );
    }

    #[test]
    fn user_correction_cluster_emits_proposal_at_threshold() {
        let events = vec![
            make_event(LearningEventType::UserCorrected, "rule-1", "ws-1"),
            make_event(LearningEventType::UserCorrected, "rule-1", "ws-1"),
            make_event(LearningEventType::UserCorrected, "rule-1", "ws-1"),
        ];

        let proposals = generate_proposals(&events);

        assert_eq!(proposals.len(), 1);
        assert_eq!(
            proposals[0].manifest_kind,
            BehaviourManifestKind::BehaviouralRuleAdjustment
        );
        assert_eq!(proposals[0].evidence_ids.len(), 3);
        assert!(proposals[0].diff.contains("\"rule_id\":\"rule-1\""));
        assert!(proposals[0].diff.contains("\"correction_count\":3"));
    }

    #[test]
    fn user_correction_cluster_below_threshold_emits_nothing() {
        let events = vec![
            make_event(LearningEventType::UserCorrected, "rule-1", "ws-1"),
            make_event(LearningEventType::UserCorrected, "rule-1", "ws-1"),
        ];

        let proposals = generate_proposals(&events);

        assert!(proposals.is_empty());
    }

    #[test]
    fn user_correction_cluster_outside_window_does_not_fire() {
        let old =
            (chrono::Utc::now() - chrono::Duration::days(PROPOSAL_WINDOW_DAYS + 1)).to_rfc3339();
        let events = vec![
            make_event_at(
                LearningEventType::UserCorrected,
                "rule-1",
                "ws-1",
                old.clone(),
            ),
            make_event_at(
                LearningEventType::UserCorrected,
                "rule-1",
                "ws-1",
                old.clone(),
            ),
            make_event_at(LearningEventType::UserCorrected, "rule-1", "ws-1", old),
        ];

        let proposals = generate_proposals(&events);

        assert!(proposals.is_empty());
    }

    #[test]
    fn user_correction_cluster_separate_rules_do_not_merge() {
        let events = vec![
            make_event(LearningEventType::UserCorrected, "rule-a", "ws-1"),
            make_event(LearningEventType::UserCorrected, "rule-a", "ws-1"),
            make_event(LearningEventType::UserCorrected, "rule-b", "ws-1"),
            make_event(LearningEventType::UserCorrected, "rule-b", "ws-1"),
        ];

        let proposals = generate_proposals(&events);

        assert!(proposals.is_empty());
    }

    #[test]
    fn user_correction_cluster_with_empty_rule_id_is_skipped() {
        let events = vec![
            make_event(LearningEventType::UserCorrected, "", "ws-1"),
            make_event(LearningEventType::UserCorrected, "", "ws-1"),
            make_event(LearningEventType::UserCorrected, "", "ws-1"),
        ];

        let proposals = generate_proposals(&events);

        assert!(proposals.is_empty());
    }
}
