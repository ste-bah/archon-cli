use std::collections::BTreeMap;

use archon_workflow::{
    DecompositionAttemptStateV1, DecompositionPhase, FixedDecompositionStateV1, FixedRunIdentityV1,
    SubjectDisposition, WorkflowRunKind,
};

#[test]
fn fixed_decomposition_identity_and_state_round_trip() {
    let identity = FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: "rev-1".into(),
        script_digest: "script-digest".into(),
        catalog_digest: "catalog-digest".into(),
        project_root_identity: "/project".into(),
        prd_identity: "/project/prds/PRD-X.md".into(),
        task_root_identity: "/project/tasks/PRD-X".into(),
    };
    let mut attempts = BTreeMap::new();
    attempts.insert(
        "acceptance".into(),
        DecompositionAttemptStateV1 {
            logical_attempt: 2,
            interrupted: false,
            last_error: Some("candidate rejected".into()),
        },
    );
    let mut dispositions = BTreeMap::new();
    dispositions.insert("acceptance".into(), SubjectDisposition::Accepted);
    let state = FixedDecompositionStateV1 {
        schema_version: 1,
        run_kind: WorkflowRunKind::FixedDecompositionV1,
        identity,
        phase: DecompositionPhase::Acceptance,
        attempts,
        dispositions,
        log_path: "/project/tasks/PRD-X/.decompose.log".into(),
    };

    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("\"run_kind\":\"fixed_decomposition_v1\""));
    assert!(json.contains("\"phase\":\"acceptance\""));
    assert_eq!(
        serde_json::from_str::<FixedDecompositionStateV1>(&json).unwrap(),
        state
    );
}

#[test]
fn workflow_run_kind_is_closed() {
    for (json, expected) in [
        (
            "\"authored_task_workflow\"",
            WorkflowRunKind::AuthoredTaskWorkflow,
        ),
        ("\"legacy_decomposed\"", WorkflowRunKind::LegacyDecomposed),
        (
            "\"fixed_decomposition_v1\"",
            WorkflowRunKind::FixedDecompositionV1,
        ),
        (
            "\"fixed_or_saved_script\"",
            WorkflowRunKind::FixedOrSavedScript,
        ),
    ] {
        assert_eq!(
            serde_json::from_str::<WorkflowRunKind>(json).unwrap(),
            expected
        );
    }
    assert!(serde_json::from_str::<WorkflowRunKind>("\"future_kind\"").is_err());
}

#[test]
fn old_generated_metadata_can_omit_run_kind() {
    #[derive(Debug, serde::Deserialize)]
    struct CompatibleMetadata {
        #[serde(default)]
        run_kind: Option<WorkflowRunKind>,
        script_lifecycle: Option<bool>,
    }

    let old: CompatibleMetadata = serde_json::from_str(r#"{"script_lifecycle":true}"#).unwrap();
    assert!(old.run_kind.is_none());
    assert_eq!(old.script_lifecycle, Some(true));
}
