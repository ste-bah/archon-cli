use archon_observability::{AgentActivityEvent, AgentActivityKind, AgentActivityStatus};

use super::*;

fn event(id: &str, kind: AgentActivityKind, message: &str) -> AgentActivityEvent {
    let mut event =
        AgentActivityEvent::new("session", kind, AgentActivityStatus::Completed, message);
    event.event_id = id.into();
    event
}

#[test]
fn heuristic_keeps_existing_source_tree_pattern() {
    let events = vec![event(
        "e1",
        AgentActivityKind::ParentTurnCompleted,
        "there is no /god-code-sdk in this source tree",
    )];
    let candidates = heuristic_candidates(&events);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].category, "source_tree_mistake");
}

#[test]
fn llm_candidate_json_accepts_fenced_valid_evidence() {
    let events = vec![event(
        "e1",
        AgentActivityKind::ToolFailed,
        "Bash permission denied",
    )];
    let envelope = parse_candidate_envelope(
        "```json\n{\"candidates\":[{\"category\":\"tool-permission-block\",\"domain\":\"cli-behavior\",\"content\":\"Check permission mode before retrying blocked shell operations.\",\"confidence\":0.88,\"evidence_event_ids\":[\"e1\"]}]}\n```",
    )
    .expect("parse envelope");
    let candidates = validate_llm_candidates(envelope, &events);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].category, "tool_permission_block");
    assert_eq!(candidates[0].domain, "cli-behavior");
}

#[test]
fn llm_validation_rejects_missing_evidence_and_secrets() {
    let events = vec![event("e1", AgentActivityKind::ToolFailed, "tool failed")];
    let envelope = LlmCandidateEnvelope {
        candidates: vec![
            LlmCandidate {
                category: "provider_auth_failure".into(),
                domain: "provider-debugging".into(),
                content: "Never store sk-ant-secretsecretsecretsecretsecret in memory.".into(),
                confidence: 0.9,
                evidence_event_ids: vec!["e1".into()],
            },
            LlmCandidate {
                category: "planning_drift".into(),
                domain: "architecture-advice".into(),
                content:
                    "Compare stated plans with actual completed work before reporting calibration."
                        .into(),
                confidence: 0.9,
                evidence_event_ids: vec!["missing".into()],
            },
        ],
    };
    assert!(validate_llm_candidates(envelope, &events).is_empty());
}

#[test]
fn dedupe_prefers_higher_confidence() {
    let low = RetrospectiveCandidate {
        category: "a".into(),
        domain: "d".into(),
        content: "Check provider auth before starting provider-sensitive work.".into(),
        confidence: 0.60,
        evidence_event_ids: vec!["e1".into()],
    };
    let mut high = low.clone();
    high.confidence = 0.90;
    let candidates = dedupe_candidates(vec![low, high]);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].confidence, 0.90);
}
