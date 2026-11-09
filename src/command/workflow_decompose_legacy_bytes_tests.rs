use super::super::FrozenChainSnapshot;
use archon_workflow::HostCommandSubject;
use archon_workflow::task_skeleton::TaskSkeleton;

#[test]
fn legacy_skeleton_file_reserializes_byte_identically() {
    let bytes = include_bytes!("../../tests/fixtures/r18_plan/task-skeleton.json");
    let skeleton: TaskSkeleton = serde_json::from_slice(bytes).unwrap();
    assert!(
        skeleton
            .tasks
            .iter()
            .all(|t| t.requirement_states.is_empty() && t.proven_done.is_empty())
    );
    assert_eq!(serde_json::to_vec_pretty(&skeleton).unwrap(), bytes);
}

#[test]
fn recorded_journal_subject_transport_bytes_are_identical() {
    let journal: serde_json::Value = serde_json::from_slice(include_bytes!("../../tests/fixtures/skeleton_resume/v2/results/2b6a852b51d8e59e8665eaf282d64683c0f9013b09abbb1e9541e66fc864a4d7-553b3e26fca0c589a126bfaa964f99a87152f1090cc2b315e3eb2adaf34877c1.json")).unwrap();
    let subjects = journal["result"]["data"]["subjects"].as_array().unwrap();
    assert!(!subjects.is_empty());
    for value in subjects {
        let bytes = serde_json::to_vec(value).unwrap();
        let typed: HostCommandSubject = serde_json::from_slice(&bytes).unwrap();
        assert!(typed.requirement_states.is_empty());
        assert!(typed.host_record_digest.is_none());
        // Journal subjects are transported through a JSON value, as in host outcomes.
        assert_eq!(
            serde_json::to_vec(&serde_json::to_value(typed).unwrap()).unwrap(),
            bytes
        );
    }
}

#[test]
fn recorded_frozen_chain_state_file_keeps_every_byte() {
    let bytes = include_bytes!(
        "../../tests/fixtures/issue349_pre_binding_fixed_run/decomposition/arguments.json"
    );
    let mut state: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let chain: FrozenChainSnapshot = serde_json::from_value(state["frozenChain"].clone()).unwrap();
    state["frozenChain"] = chain.to_argument();
    let mut serialized = serde_json::to_vec_pretty(&state).unwrap();
    serialized.push(b'\n');
    assert_eq!(serialized, bytes);
}

#[test]
fn recorded_decomposition_state_file_keeps_every_byte() {
    let bytes = include_bytes!(
        "../../tests/fixtures/issue349_pre_binding_fixed_run/decomposition/state.json"
    );
    let state: archon_workflow::FixedDecompositionStateV1 = serde_json::from_slice(bytes).unwrap();
    let mut serialized = serde_json::to_vec_pretty(&state).unwrap();
    serialized.push(b'\n');
    assert_eq!(serialized, bytes);
}
