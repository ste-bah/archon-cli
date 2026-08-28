use super::*;

pub(crate) fn fixed_attempt(project: &Path, run_id: &str, subject: &str) -> u64 {
    let store = archon_workflow::WorkflowStore::project(project);
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json")).unwrap(),
    )
    .unwrap();
    state["attempts"][subject]["logical_attempt"]
        .as_u64()
        .expect("persisted logical attempt")
}

pub(crate) fn assert_acceptance_reused(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    assert!(events.iter().any(|event| {
        event["detail"]["phase"] == "acceptance" && event["detail"]["reused"] == true
    }));
}

pub(crate) fn assert_fixed_provider_route(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let route: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            store
                .run_dir(run_id)
                .join("decomposition/provider-route.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(route["origin"], "trusted_config");
    assert!(route["endpoint_digest"].as_str().is_some());
}

pub(crate) fn assert_decomposition_events(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    for expected in [
        "author_attempt_started",
        "author_attempt_completed",
        "host_command_started",
        "host_command_completed",
        "decomposition_completed",
    ] {
        assert!(
            events.iter().any(|event| event["kind"] == expected),
            "missing {expected}: {events:#?}"
        );
    }
    let raw = std::fs::read_to_string(store.events_path(run_id)).unwrap();
    assert!(!raw.contains("candidate"));
    assert!(!raw.contains("ANTHROPIC"));
}

pub(crate) fn assert_frozen_floor(
    project: &Path,
    run_id: &str,
    expected: &archon_workflow::task_universe::WorkflowV2DeliverableContract,
) {
    let store = archon_workflow::WorkflowStore::project(project);
    let fixed: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json")).unwrap(),
    )
    .unwrap();
    let task_root = PathBuf::from(fixed["identity"]["task_root_identity"].as_str().unwrap());
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&std::fs::read(task_root.join("acceptance-contract.json")).unwrap())
            .unwrap();
    let criterion = contract
        .acceptance
        .iter()
        .find(|criterion| criterion.id == "AC-SYN-001")
        .unwrap();
    match &criterion.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Floor { contract } => {
            assert_eq!(contract, expected);
            assert!(contract.typed_verifier_command.is_none());
        }
        archon_workflow::task_set_contract::AcceptanceCheck::Command { .. } => {
            panic!("synthetic proof refuses command-bearing frozen acceptance")
        }
    }
}

pub(crate) fn assert_synthetic_outputs(project: &Path) {
    assert_eq!(
        std::fs::read_to_string(project.join("src/alpha.txt"))
            .unwrap()
            .trim(),
        "alpha ready"
    );
    let beta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(project.join("src/beta.json")).unwrap()).unwrap();
    assert_eq!(beta["ready"], true);
    assert_eq!(beta.as_object().map(serde_json::Map::len), Some(1));
    assert!(
        !project
            .join(".archon/proof/synthetic-observer-target.json")
            .exists()
    );
}

pub(crate) fn assert_observer_after_terminal(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    let terminal_seq = events
        .iter()
        .filter(|event| event["kind"] == "completed")
        .filter_map(|event| event["seq"].as_u64())
        .max()
        .expect("terminal event");
    let observer_seq = events
        .iter()
        .filter(|event| {
            matches!(
                event["kind"].as_str(),
                Some("run_end_acceptance_observer_started")
                    | Some("run_end_acceptance_shadow_observed")
            )
        })
        .filter_map(|event| event["seq"].as_u64())
        .min()
        .expect("observer event");
    assert!(terminal_seq < observer_seq);
    let finalization: archon_workflow::FinalizationRecordV1 = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("v2/finalization.json")).unwrap(),
    )
    .unwrap();
    assert!(finalization.terminal_state_committed);
    assert!(finalization.terminal_event_committed);
    match finalization.observer_state {
        Some(archon_workflow::RunEndObserverStateV1::Completed { outcome }) => {
            assert_eq!(
                outcome.authority,
                archon_workflow::ObserverAuthority::ObserveOnly
            );
            assert!(outcome.evaluated_floor_count >= 1);
            assert!(outcome.policy_finding_count >= 1);
        }
        other => panic!("observer did not complete with shadow findings: {other:?}"),
    }
    let records = store
        .run_dir(run_id)
        .join("observer/run-end-acceptance.jsonl");
    let text = std::fs::read_to_string(records).unwrap();
    assert!(text.contains("AC-SYN-001"));
    assert!(text.contains("observe_only"));
}
