use super::*;

#[test]
fn binding_states_overwrite_authored_values_and_host_line_is_idempotent() {
    use archon_workflow::requirement_state::{RequirementDecision, RequirementState};
    let mut frozen = frozen_with_owned_fields();
    frozen.requirement_states.insert(
        "AC-X-001".into(),
        RequirementDecision::new(RequirementState::Undecided, "weak_test"),
    );
    frozen.status = Some("kept".into());
    let body = "```yaml\ntask_id: TASK-X-010\ntitle: item\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\nrequirement_states: nonsense\n```\n\nBody\n";
    let (first, notes) =
        crate::command::workflow_host_command_binding::stamp_frozen_frontmatter(body, &frozen)
            .unwrap();
    let (second, _) =
        crate::command::workflow_host_command_binding::stamp_frozen_frontmatter(&first, &frozen)
            .unwrap();
    assert_eq!(first, second);
    assert!(notes.iter().any(|n| n.contains("requirement_states")));
    assert_eq!(first.matches("Host classification (").count(), 1);
    assert!(first.contains("kept, not rebuilt: AC-X-001"));
    let parsed = archon_workflow::task_universe::parsing::parse_task_file(
        std::path::Path::new("TASK-X-010.md"),
        &first,
    )
    .unwrap();
    assert_eq!(parsed.requirement_states["AC-X-001"], "undecided");
    assert!(archon_workflow::task_skeleton::compare_frozen_task(&parsed, &frozen).is_empty());
    let bad = first.replace("undecided", "invented");
    assert!(
        archon_workflow::task_universe::parsing::parse_task_file(
            std::path::Path::new("TASK-X-010.md"),
            &bad
        )
        .is_err()
    );
    let mut changed = parsed;
    changed
        .requirement_states
        .insert("AC-X-001".into(), "done".into());
    assert!(
        archon_workflow::task_skeleton::compare_frozen_task(&changed, &frozen)
            .iter()
            .any(|f| f.field == "requirement_states")
    );
}

#[test]
fn legacy_subject_bytes_and_host_evidence_digest_are_stable() {
    let raw = br#"{"taskId":"TASK-X-010","fileName":"TASK-X-010.md"}"#;
    let subject: archon_workflow::HostCommandSubject = serde_json::from_slice(raw).unwrap();
    assert_eq!(serde_json::to_vec(&subject).unwrap(), raw);
    let mut frozen = frozen_with_owned_fields();
    let legacy = archon_workflow::HostCommandSubject::from_frozen(&frozen);
    assert!(legacy.host_record_digest.is_none());
    frozen.requirement_states.insert(
        "AC-X-001".into(),
        archon_workflow::requirement_state::RequirementDecision::new(
            archon_workflow::requirement_state::RequirementState::Undecided,
            "weak_test",
        ),
    );
    let first = archon_workflow::HostCommandSubject::from_frozen(&frozen);
    frozen
        .requirement_states
        .get_mut("AC-X-001")
        .unwrap()
        .reason = "no_viable_behaviour_mutant".into();
    assert_ne!(
        first.host_record_digest,
        archon_workflow::HostCommandSubject::from_frozen(&frozen).host_record_digest
    );
}

#[test]
fn recorded_frozen_state_and_legacy_skeleton_reserialize_byte_identically() {
    let bytes = include_bytes!(
        "../../tests/fixtures/issue349_pre_binding_fixed_run/decomposition/state.json"
    );
    let state: archon_workflow::FixedDecompositionStateV1 = serde_json::from_slice(bytes).unwrap();
    let mut encoded_state = serde_json::to_vec_pretty(&state).unwrap();
    encoded_state.push(b'\n');
    assert_eq!(encoded_state, bytes);
    let bytes = include_bytes!("../../tests/fixtures/r18_plan/task-skeleton.json");
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(bytes).unwrap();
    assert_eq!(serde_json::to_vec_pretty(&skeleton).unwrap(), bytes);
}
