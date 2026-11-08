use super::*;

#[test]
fn skeleton_preparation_does_not_execute_frozen_checks() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    contract.prd.digest = content_digest(&std::fs::read(&prd).unwrap());
    let marker = temp.path().join("check-executed");
    contract.acceptance[0].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: format!("echo executed > \"{}\"", marker.display()),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    contract.acceptance[0].judgment.verdict =
        archon_workflow::task_set_contract::JudgeDecision::Accepted;
    let bytes = serde_json::to_vec(&contract).unwrap();
    let (lock, pin) = findings::acceptance_lock_and_pin(
        &tasks,
        archon_workflow::task_set_contract::FreezeGateMode::Enforce,
        &[],
        &bytes,
        None,
    );
    std::fs::write(&path, bytes).unwrap();
    std::fs::write(
        tasks.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec(&lock).unwrap(),
    )
    .unwrap();
    let pin_path = acceptance_pin_path(temp.path(), &tasks);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-skeleton");
    let prepared = prepare_skeleton_freeze_from_candidate_with_resume(
        temp.path(), &tasks, &prd, GateMode::Enforce,
        br#"{"schema_version":1,"acceptance_digest":"pending","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"],"source_sections":["0"]}]}"#.to_vec(),
        &resume,
    ).unwrap();
    assert!(prepared.findings.is_empty(), "{:?}", prepared.findings);
    assert!(
        !marker.exists(),
        "skeleton preparation executed an acceptance check"
    );
}

#[test]
fn skeleton_preparation_does_not_execute_frozen_checks_or_trust_authored_completion() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let prd = temp.path().join("prds/PRD-X.md");
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(), &tasks, &prd, GateMode::Observe,
        br#"{"schema_version":1,"acceptance_digest":"pending","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"],"source_sections":["0"],"status":"done"}]}"#.to_vec(),
    ).unwrap();
    let frozen: TaskSkeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
    assert_eq!(frozen.tasks[0].status, None);
    assert!(
        serde_json::to_value(&frozen).unwrap()["tasks"][0]
            .get("status")
            .is_none()
    );
}

#[test]
fn skeleton_preparation_discards_non_string_authored_status_without_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let prd = temp.path().join("prds/PRD-X.md");
    for status in [
        serde_json::json!(1),
        serde_json::json!(true),
        serde_json::json!(["done"]),
        serde_json::json!({"value": "done"}),
        serde_json::Value::Null,
    ] {
        let candidate = serde_json::json!({"schema_version":1,"acceptance_digest":"pending","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"],"source_sections":["0"],"status":status}]});
        let bytes = serde_json::to_vec(&candidate).unwrap();
        let prepared = prepare_skeleton_freeze_from_candidate(
            temp.path(),
            &tasks,
            &prd,
            GateMode::Observe,
            bytes,
        )
        .unwrap();
        assert!(prepared.findings.is_empty(), "{:?}", prepared.findings);
        let frozen: TaskSkeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
        assert_eq!(frozen.tasks[0].status, None);
        assert!(
            serde_json::to_value(&frozen).unwrap()["tasks"][0]
                .get("status")
                .is_none()
        );
    }
}

#[test]
fn skeleton_preparation_clears_unvalidated_deliverable_implements() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let prd = temp.path().join("prds/PRD-X.md");
    let candidate = serde_json::json!({"schema_version":1,"acceptance_digest":"pending","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"],"source_sections":["0"],"deliverable_contracts":[{"kind":"report","artifact_path":"out.json","min_instances":1,"implements":["invented"]}]}]});
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(),
    )
    .unwrap();
    let frozen: TaskSkeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
    assert!(
        frozen.tasks[0].deliverable_contracts[0]
            .implements
            .is_empty()
    );
    assert!(
        !String::from_utf8(prepared.skeleton_bytes)
            .unwrap()
            .contains("invented")
    );
}

#[test]
fn skeleton_preparation_acceptance_floor_clears_unvalidated_implements() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, candidate) = seed(&temp);
    let text = std::fs::read_to_string(&prd).unwrap();
    let criteria = acceptance_criteria(&text);
    let expected = criteria.keys().cloned().collect();
    let mut candidate: serde_json::Value = serde_json::from_slice(&candidate).unwrap();
    candidate["acceptance"][0]["check"] = serde_json::json!({"kind":"floor","contract":{"kind":"report","artifact_path":"out.json","typed_verifier_command":"jq -e .valid out.json","implements":["invented"]}});
    candidate["supplementary"] = serde_json::json!([{"id":"SUP-extra","criterion":"extra","check":candidate["acceptance"][0]["check"].clone(),"gap_permitted":false,"judgment":candidate["acceptance"][0]["judgment"].clone()}]);
    let prepared = preflight::prepare(
        temp.path(),
        &tasks,
        &prd,
        "digest",
        &text,
        &criteria,
        &expected,
        &serde_json::to_vec(&candidate).unwrap(),
    )
    .unwrap();
    for entry in prepared.acceptance.iter().chain(&prepared.supplementary) {
        let archon_workflow::task_set_contract::AcceptanceCheck::Floor { contract } = &entry.check
        else {
            panic!("expected floor")
        };
        assert!(contract.implements.is_empty());
    }
}
