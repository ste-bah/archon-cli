use super::call_metadata::{CallKind, CallMetadata, CallSemantics};
use super::completion_evidence::task_completion_evidence_kind;
use super::host_api::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};
use super::result_store::WorkflowV2TaskCompletionEvidenceKind as EvidenceKind;
use serde_json::json;

pub(super) fn call(id: &str, kind: CallKind) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: WorkflowV2HostOptions {
            metadata: Some(CallMetadata {
                kind,
                task_scoped: true,
                sequence: Some(7),
                source_graph: None,
            }),
            ..Default::default()
        },
    }
}

#[test]
fn renamed_workflow_preserves_typed_completion_and_commands() {
    for (kind, expected, commands) in [
        (CallKind::Other, None, false),
        (CallKind::NoopProof, Some(EvidenceKind::VerifiedNoop), false),
        (
            CallKind::Implementation,
            Some(EvidenceKind::ImplementationCandidate),
            false,
        ),
        (
            CallKind::Remediation,
            Some(EvidenceKind::ImplementationCandidate),
            false,
        ),
        (
            CallKind::ReviewRemediation,
            Some(EvidenceKind::ImplementationCandidate),
            false,
        ),
        (
            CallKind::FocusedVerification,
            Some(EvidenceKind::FocusedVerification),
            true,
        ),
        (
            CallKind::ReviewVerification,
            Some(EvidenceKind::FocusedVerification),
            true,
        ),
    ] {
        for id in [
            "custom-step",
            "verification-wave-misleading",
            "implement-task-misleading",
        ] {
            let step = call(id, kind);
            assert_eq!(task_completion_evidence_kind(&step), expected);
            assert_eq!(step.verification(), commands);
            assert_eq!(
                crate::stage_command_policy::v2_call_runs_commands(id, &json!({"v2_call": step})),
                commands
            );
        }
    }
}

#[test]
fn old_records_keep_prefix_behavior_and_metadata_round_trips() {
    let mut old = call("verification-wave-old", CallKind::Other);
    old.options.metadata = None;
    assert!(old.verification());
    old.options.item_kind = Some("implementation".into());
    assert!(!old.verification());
    let typed = call("different", CallKind::FocusedVerification);
    let restored: WorkflowV2HostCall =
        serde_json::from_value(serde_json::to_value(&typed).unwrap()).unwrap();
    assert_eq!(restored, typed);
    assert_eq!(restored.metadata().sequence, Some(7));
}

#[test]
fn prose_never_grants_shell_access() {
    assert!(!crate::stage_command_policy::v2_call_runs_commands(
        "renamed",
        &json!({"task": "verify and run tests"})
    ));
    assert!(crate::stage_command_policy::v2_call_runs_commands(
        "renamed",
        &json!({"stage_extra": {"required_tools": ["shell"]}})
    ));
}
