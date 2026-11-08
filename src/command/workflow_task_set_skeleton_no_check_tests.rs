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
        None.into(),
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
fn skeleton_preparation_refuses_unclaimed_deliverable_implements() {
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
    let finding = prepared
        .findings
        .iter()
        .find(|f| {
            f.deterministic_defect
                .as_ref()
                .is_some_and(|d| d.code == "deliverable_implements_unclaimed")
        })
        .expect("unclaimed deliverable id must have a finding");
    assert!(finding.text.contains("does not claim in implements"));
    let disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        GateMode::Enforce,
        GateId::FreezeSkeleton,
        || {
            Ok(crate::command::workflow_gate::GateEvaluation::new(
                "",
                prepared.findings,
            ))
        },
    )
    .unwrap();
    assert!(
        disposition.require_allowed().is_err(),
        "an unclaimed id cannot pass the gate"
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

#[test]
fn s4a_valid_deliverable_claims_survive_freeze_stamp_and_parse() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let prd = temp.path().join("prds/PRD-X.md");
    let candidate = serde_json::json!({"schema_version":1,"acceptance_digest":"pending","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"],"source_sections":["0"],"deliverable_contracts":[{"kind":"report","artifact_path":"out.json","min_instances":1,"implements":["AC-X-001"]}]}]});
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(),
    )
    .unwrap();
    let frozen: TaskSkeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
    assert_eq!(
        frozen.tasks[0].deliverable_contracts[0].implements,
        ["AC-X-001"]
    );
    let body = "```yaml\ntask_id: TASK-X-010\ntitle: Body\ncomplexity: low\nstatus: ready\nrequired_env_keys: []\nrequired_tools: []\n```\n\n# Body\n\n## Focused Tests\n- `test -f out.json`\n";
    let (stamped, _) = crate::command::workflow_host_command_binding::stamp_frozen_frontmatter(
        body,
        &frozen.tasks[0],
    )
    .unwrap();
    let parsed = archon_workflow::task_universe::parsing::parse_task_file(
        &tasks.join("TASK-X-010.md"),
        &stamped,
    )
    .unwrap();
    assert_eq!(parsed.deliverable_contracts[0].implements, ["AC-X-001"]);
    let identity = prepared.publication_identity();
    let findings = prepared.findings.clone();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        GateMode::Observe,
        GateId::FreezeSkeleton,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(identity),
            )
        },
    )
    .unwrap();
    publish_skeleton_freeze(prepared, disposition.take_publication_permit().unwrap()).unwrap();
    let published: TaskSkeleton =
        serde_json::from_slice(&std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap()).unwrap();
    assert_eq!(
        published.tasks[0].deliverable_contracts[0].implements,
        ["AC-X-001"]
    );
}

#[test]
fn s4a_freezes_in_different_temp_roots_have_identical_lock_bytes() {
    use archon_workflow::repository_record::{RepositoryRecordV1, write_repository_record};
    fn freeze(root: &tempfile::TempDir, scenario: &str) -> Vec<u8> {
        let (tasks, prd, _) = seed(root);
        let mut contract: AcceptanceContract =
            serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
                .unwrap();
        contract.prd.digest = content_digest(&std::fs::read(&prd).unwrap());
        contract.acceptance[0].judgment.verdict =
            archon_workflow::task_set_contract::JudgeDecision::Accepted;
        let bytes = serde_json::to_vec_pretty(&contract).unwrap();
        let base = if scenario == "present" {
            "unborn"
        } else {
            "invalid-commit"
        };
        let (lock, pin) = findings::acceptance_lock_and_pin(
            &tasks,
            archon_workflow::task_set_contract::FreezeGateMode::Observe,
            &[],
            &bytes,
            Some(base.into()).into(),
        );
        std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
        std::fs::write(
            tasks.join(ACCEPTANCE_LOCK_FILE),
            serde_json::to_vec(&lock).unwrap(),
        )
        .unwrap();
        let pin_path = acceptance_pin_path(root.path(), &tasks);
        std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
        std::fs::write(pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();
        let repository_root = if scenario == "missing_root" {
            root.path().join("missing")
        } else {
            root.path().to_path_buf()
        };
        let record = RepositoryRecordV1 {
            schema_version: 1,
            repository_root: repository_root.to_string_lossy().into(),
            base_commit: "unborn".into(),
            decomposition_run_id: "fixed".into(),
            recorded_at: root.path().to_string_lossy().into(),
        };
        // The freeze's ownership validation needs a readable repository record.
        let mut readable = record.clone();
        readable.repository_root = root.path().to_string_lossy().into();
        write_repository_record(&tasks, &readable).unwrap();
        let candidate = br#"{"schema_version":1,"acceptance_digest":"pending","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"],"source_sections":["0"],"deliverable_contracts":[{"kind":"output","artifact_path":"out.json","min_instances":1,"implements":["AC-X-001"]}]}]}"#.to_vec();
        // Hold the supplied audit stamp fixed: wall-clock metadata is a separate input.
        fn fixed_stamp(
            mode: archon_workflow::task_set_contract::FreezeGateMode,
            findings: &[crate::command::workflow_gate::GateFinding],
        ) -> archon_workflow::task_set_contract::FreezeGateStamp {
            let mut stamp = gate_stamp(mode, findings);
            stamp.evaluated_at = "2026-01-01T00:00:00Z".into();
            stamp
        }
        let mut prepared = super::super::skeleton_freeze::prepare_with_diagnostics(
            root.path(),
            &tasks,
            &prd,
            GateMode::Observe,
            candidate,
            super::super::link_diagnostics::child,
            fixed_stamp,
        )
        .unwrap();
        if scenario != "present" {
            // Exercise serialized error diagnostics separately from the earlier ownership gate.
            write_repository_record(&tasks, &record).unwrap();
            if scenario == "malformed_record" {
                std::fs::write(
                    archon_workflow::repository_record::repository_record_path(&tasks),
                    b"{",
                )
                .unwrap();
            }
            let skeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
            prepared.lock.classification = Some(findings::classification_from_lock(
                &tasks, &skeleton, &contract,
            ));
        }
        let record = prepared.lock.classification.as_ref().unwrap();
        assert_eq!(record.error.is_none(), scenario == "present", "{record:?}");
        let bytes = serde_json::to_vec_pretty(&prepared.lock).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(root.path().to_str().unwrap()));
        bytes
    }
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for scenario in [
        "present",
        "missing_root",
        "invalid_commit",
        "malformed_record",
    ] {
        // A fresh task root for each freeze keeps previous publications out of the inputs.
        let a = tempfile::tempdir_in(first.path()).unwrap();
        let b = tempfile::tempdir_in(second.path()).unwrap();
        assert_eq!(freeze(&a, scenario), freeze(&b, scenario), "{scenario}");
    }
}
