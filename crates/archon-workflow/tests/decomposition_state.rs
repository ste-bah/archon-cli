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

#[test]
fn fixed_resume_identity_requires_every_immutable_component() {
    use archon_workflow::verify_fixed_resume_identity;

    let baseline = FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: "rev-1".into(),
        script_digest: "script-1".into(),
        catalog_digest: "catalog-1".into(),
        project_root_identity: "/project".into(),
        prd_identity: "/project/PRD.md".into(),
        task_root_identity: "/project/tasks".into(),
    };
    verify_fixed_resume_identity(&baseline, &baseline).unwrap();

    for (field, changed) in [
        (
            "template_version",
            FixedRunIdentityV1 {
                template_version: "fixed-decomposition-v2".into(),
                ..baseline.clone()
            },
        ),
        (
            "starting_binary_revision",
            FixedRunIdentityV1 {
                starting_binary_revision: "rev-2".into(),
                ..baseline.clone()
            },
        ),
        (
            "script_digest",
            FixedRunIdentityV1 {
                script_digest: "script-2".into(),
                ..baseline.clone()
            },
        ),
        (
            "catalog_digest",
            FixedRunIdentityV1 {
                catalog_digest: "catalog-2".into(),
                ..baseline.clone()
            },
        ),
        (
            "project_root_identity",
            FixedRunIdentityV1 {
                project_root_identity: "/other".into(),
                ..baseline.clone()
            },
        ),
        (
            "prd_identity",
            FixedRunIdentityV1 {
                prd_identity: "/project/other.md".into(),
                ..baseline.clone()
            },
        ),
        (
            "task_root_identity",
            FixedRunIdentityV1 {
                task_root_identity: "/project/other-tasks".into(),
                ..baseline.clone()
            },
        ),
    ] {
        let error = verify_fixed_resume_identity(&baseline, &changed).unwrap_err();
        assert!(error.to_string().contains(field), "{field}: {error}");
        assert!(error.to_string().contains("do not deploy"), "{error}");
    }
}
