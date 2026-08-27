use archon_workflow::{
    GATE_ENVELOPE_SCHEMA_VERSION, GateEnvelopeV1, GateOperationalError, GatePolicyFinding,
    RemediationScope,
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
                text: format!("exact {scope:?} finding"),
                subject: "TASK-X-010".into(),
                source_path: Some("tasks/PRD-X/TASK-X-010.md".into()),
                remediation_scope: scope,
            })
            .collect(),
        operational_error: Some(GateOperationalError {
            kind: "test_operational".into(),
            text: "exact operational failure".into(),
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
