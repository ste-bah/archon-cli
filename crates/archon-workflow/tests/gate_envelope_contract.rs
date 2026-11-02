use archon_workflow::{
    GATE_ENVELOPE_SCHEMA_VERSION, GateEnvelopeV1, GateOperationalError, GatePolicyFinding,
    GateStage, RemediationScope,
};

#[test]
fn gate_envelope_round_trips_every_closed_remediation_scope() {
    let scopes = [
        RemediationScope::CandidateArtifact,
        RemediationScope::Skeleton,
        RemediationScope::PrdInput,
        RemediationScope::Body,
        RemediationScope::InheritedPredecessor,
        RemediationScope::Operational,
    ];
    let envelope = GateEnvelopeV1 {
        schema_version: GATE_ENVELOPE_SCHEMA_VERSION,
        report: serde_json::json!({"kind": "test-report"}),
        policy_findings: scopes
            .into_iter()
            .map(|scope| GatePolicyFinding {
                deterministic_defect: None,
                text: format!("exact {scope:?} finding"),
                subject: "TASK-X-010".into(),
                source_path: Some("tasks/PRD-X/TASK-X-010.md".into()),
                remediation_scope: scope,
                gate_stage: None,
                judged_item: None,
            })
            .collect(),
        operational_error: Some(GateOperationalError {
            kind: "test_operational".into(),
            text: "exact operational failure".into(),
            subjects: Vec::new(),
        }),
    };

    let encoded = serde_json::to_value(&envelope).unwrap();
    assert_eq!(encoded["schema_version"], 1);
    assert_eq!(
        encoded["policy_findings"][4]["remediation_scope"],
        "inherited_predecessor"
    );
    assert_eq!(
        serde_json::from_value::<GateEnvelopeV1>(encoded).unwrap(),
        envelope
    );
}

#[test]
fn gate_envelope_rejects_unknown_remediation_scope() {
    let error = serde_json::from_value::<GateEnvelopeV1>(serde_json::json!({
        "schema_version": 1,
        "report": {},
        "policy_findings": [{
            "text": "finding",
            "subject": "subject",
            "source_path": null,
            "remediation_scope": "guess_from_prose"
        }],
        "operational_error": null
    }))
    .expect_err("unknown scope must fail closed");

    assert!(error.to_string().contains("unknown variant"), "{error}");
}

/// #562: a finding's gate stage and judged item are optional on the wire, so
/// an envelope written before them reads unchanged and writes no new keys.
#[test]
fn gate_stage_and_judged_item_are_optional_on_the_wire() {
    let recorded = serde_json::json!({
        "schema_version": 1,
        "report": {},
        "policy_findings": [{
            "text": "finding",
            "subject": "TASK-X-010",
            "remediation_scope": "body"
        }]
    });
    let envelope = serde_json::from_value::<GateEnvelopeV1>(recorded.clone()).unwrap();
    assert_eq!(envelope.policy_findings[0].gate_stage, None);
    assert_eq!(envelope.policy_findings[0].judged_item, None);
    assert_eq!(serde_json::to_value(&envelope).unwrap(), recorded);

    let mut staged = envelope;
    staged.policy_findings[0].gate_stage = Some(GateStage::Critic);
    staged.policy_findings[0].judged_item = Some("REQ-X-001".into());
    let encoded = serde_json::to_value(&staged).unwrap();
    assert_eq!(encoded["policy_findings"][0]["gate_stage"], "critic");
    assert_eq!(encoded["policy_findings"][0]["judged_item"], "REQ-X-001");
    assert_eq!(
        serde_json::from_value::<GateEnvelopeV1>(encoded).unwrap(),
        staged
    );
}
