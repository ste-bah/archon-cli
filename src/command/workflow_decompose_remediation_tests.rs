use super::*;

#[path = "workflow_decompose_remediation_lineage_tests.rs"]
mod lineage_tests;

#[path = "workflow_decompose_remediation_read_tests.rs"]
mod read_tests;
#[path = "workflow_decompose_remediation_scope_tests.rs"]
mod scope_tests;

#[tokio::test]
async fn check_ledger_subject_reauthors_even_when_a_task_implements_it_with_finding_seed() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "jq -e '.a == true' out.json",
        true,
    )]);
    let run_dir = tempfile::tempdir().unwrap();
    let report_dir = run_dir.path().join("decomposition/reverify/reports");
    std::fs::create_dir_all(&report_dir).unwrap();
    let finding = "passed on base but the can-fail proof has no named input";
    std::fs::write(
        report_dir.join("report-check-defect.json"),
        serde_json::json!({
            "checks": [{"check_id":"AC-F-001", "findings":[finding]}]
        })
        .to_string(),
    )
    .unwrap();
    let ledger = Ledger {
        report_id: "report-check-defect".into(),
        subjects: vec!["AC-F-001".into()],
    };
    let client = crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
        |entry, _| {
            crate::command::workflow_task_set::reauthor::test_client::command_entry(
                entry,
                "jq -e '.a == true and .b == 1' out.json",
            )
        },
        |_, _| true,
    );

    reauthor_check_subjects_on_resume(
        (set.project.path(), &set.prd, &set.tasks, run_dir.path()),
        Some(&ledger),
        &client,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();

    assert_eq!(client.authored(), 1);
    assert!(
        client
            .judged_ids
            .lock()
            .unwrap()
            .iter()
            .all(|id| id == "AC-F-001")
    );
    assert!(client.prompts.lock().unwrap()[0].contains(finding));
}

#[test]
fn completed_run_with_ledger_builds_seed_and_other_cases_do_not() {
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "seed-test".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let runtime = archon_workflow::FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: "revision".into(),
        script_digest: "script".into(),
        catalog_digest: "catalog".into(),
        project_root_identity: "/project".into(),
        prd_identity: "/project/PRD.md".into(),
        task_root_identity: "/project/tasks".into(),
    };
    let chain = serde_json::json!({"acceptance": false, "skeleton": false, "bodies": []});
    let criteria = std::collections::BTreeMap::new();
    let requirement_texts = std::collections::BTreeMap::new();
    let ledger = Ledger {
        report_id: "report".into(),
        subjects: vec!["TASK-A-001".into()],
    };
    let history = archon_workflow::WorkflowV2CallRecord::new(
        &run.id,
        archon_workflow::WorkflowV2HostCall {
            id: "body-TASK-A-001-author-1".into(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: archon_workflow::WorkflowV2HostOptions::default(),
        },
        1,
        "body-input".into(),
        archon_workflow::WorkflowV2Result::accepted("prior body"),
        Vec::new(),
    );
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .save_call_record(&history)
        .unwrap();

    let mut pending = store.load_state(&run.id).unwrap();
    store.save_state(&pending).unwrap();
    assert!(
        crate::command::workflow_decompose_seed::completed_remediation_seed(
            &store,
            &run.id,
            &runtime,
            &criteria,
            &requirement_texts,
            &chain,
            Some(&ledger),
        )
        .unwrap()
        .is_none()
    );

    pending.status = archon_workflow::RunStatus::Paused;
    store.save_state(&pending).unwrap();
    let paused_seed = crate::command::workflow_decompose_seed::completed_remediation_seed(
        &store,
        &run.id,
        &runtime,
        &criteria,
        &requirement_texts,
        &chain,
        Some(&ledger),
    )
    .unwrap()
    .expect("paused remediation findings also need a phase seed");
    assert_eq!(paused_seed.resume_frozen_chain.as_ref(), Some(&chain));

    pending.status = archon_workflow::RunStatus::Completed;
    store.save_state(&pending).unwrap();
    let seed = crate::command::workflow_decompose_seed::completed_remediation_seed(
        &store,
        &run.id,
        &runtime,
        &criteria,
        &requirement_texts,
        &chain,
        Some(&ledger),
    )
    .unwrap()
    .expect("completed remediation resumes need a phase seed");
    assert_eq!(seed.resume_frozen_chain.as_ref(), Some(&chain));
    assert!(seed.subjects.is_empty());
    assert_eq!(seed.author_ordinals.get("body-TASK-A-001"), Some(&1));
    assert!(
        crate::command::workflow_decompose_seed::completed_remediation_seed(
            &store,
            &run.id,
            &runtime,
            &criteria,
            &requirement_texts,
            &chain,
            None,
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn force_subjects_only_overrides_the_ledger_subjects_and_consumption_is_append_only() {
    let temp = tempfile::tempdir().unwrap();
    let mut arguments = serde_json::json!({"forceBodySubjects": ["old"]});
    let ledger = Ledger {
        report_id: "reverify-4-report".into(),
        subjects: vec!["AC-A-001".into(), "TASK-A-001".into()],
    };
    force_subjects(
        &mut arguments,
        Some(&ledger),
        &std::collections::BTreeSet::from(["TASK-A-001".into()]),
    );
    assert_eq!(
        arguments["forceBodySubjects"],
        serde_json::json!(["TASK-A-001"])
    );
    append_consumed(temp.path(), &ledger).unwrap();
    append_consumed(temp.path(), &ledger).unwrap();
    let text =
        std::fs::read_to_string(temp.path().join("decomposition/reverify/consumed.jsonl")).unwrap();
    let records = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["report_id"], "reverify-4-report");
    assert_eq!(
        records[0]["subjects"],
        serde_json::json!(["AC-A-001", "TASK-A-001"])
    );
}

#[test]
fn paused_finding_report_prepares_resume_seed_and_rejects_unknown_consumed_subjects() {
    // EXPECTED BEFORE RUN: a paused run with a persisted needs_remediation
    // report and ledger prepares a phase seed and tolerates a torn event tail.
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "resume-seed-test".into(),
            task: "resume seed test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let original = archon_workflow::WorkflowV2CallRecord::new(
        &run.id,
        archon_workflow::WorkflowV2HostCall {
            id: "final-report".into(),
            method: archon_workflow::WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "original-report".into(),
        archon_workflow::WorkflowV2Result {
            status: archon_workflow::WorkflowV2Status::Accepted,
            data: serde_json::json!({"outcome":"accepted", "subjects": []}),
            ..Default::default()
        },
        Vec::new(),
    );
    let reverify = archon_workflow::WorkflowV2CallRecord::new(
        &run.id,
        archon_workflow::WorkflowV2HostCall {
            id: "reverify-report".into(),
            method: archon_workflow::WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "reverify-report".into(),
        archon_workflow::WorkflowV2Result {
            status: archon_workflow::WorkflowV2Status::NeedsReview,
            data: serde_json::json!({
                "outcome":"needs_remediation",
                "supersedes_id":"final-report",
                "subjects":[{"task_id":"TASK-A-001"}]
            }),
            ..Default::default()
        },
        Vec::new(),
    );
    let results = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    results.save_call_record(&original).unwrap();
    results.save_call_record(&reverify).unwrap();
    let event_log = archon_workflow::WorkflowEventLog::new(store.clone());
    for (kind, detail) in [
        (
            archon_workflow::WorkflowEventKind::DecompositionCompleted,
            serde_json::json!({"call_id":"final-report"}),
        ),
        (
            archon_workflow::WorkflowEventKind::DecompositionReverified,
            serde_json::json!({"report_id":"reverify-report", "supersedes_id":"final-report"}),
        ),
    ] {
        let seq = store.next_event_seq(&run.id).unwrap();
        event_log.emit(&run.id, seq, kind, detail).unwrap();
    }
    let mut paused = store.load_state(&run.id).unwrap();
    paused.status = archon_workflow::RunStatus::Paused;
    store.save_state(&paused).unwrap();
    std::fs::create_dir_all(store.run_dir(&run.id).join("decomposition/reverify")).unwrap();
    std::fs::write(
        store
            .run_dir(&run.id)
            .join("decomposition/reverify/remediation-reverify-report.json"),
        serde_json::to_vec(&serde_json::json!({
            "report_id":"reverify-report", "subjects":["TASK-A-001"]
        }))
        .unwrap(),
    )
    .unwrap();
    let events_path = store.events_path(&run.id);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(events_path)
        .unwrap()
        .write_all(b"{\"seq\":")
        .unwrap();
    let tasks = tempfile::tempdir().unwrap();
    std::fs::write(
        tasks
            .path()
            .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        serde_json::json!({
            "schema_version": 1,
            "acceptance_digest": "digest",
            "tasks": [{
                "task_id": "TASK-A-001",
                "file_name": "TASK-A-001.md",
                "depends_on": [],
                "blocks": [],
                "implements": [],
                "deliverable_contracts": []
            }]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        tasks
            .path()
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        r#"{"schema_version":1,"prd":{"path":"PRD.md","digest":"digest"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[],"supplementary":[]}"#,
    )
    .unwrap();
    let runtime = archon_workflow::FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: "revision".into(),
        script_digest: "script".into(),
        catalog_digest: "catalog".into(),
        project_root_identity: "/project".into(),
        prd_identity: "/project/PRD.md".into(),
        task_root_identity: tasks.path().to_string_lossy().into_owned(),
    };
    let criteria = std::collections::BTreeMap::new();
    let requirement_texts = std::collections::BTreeMap::new();
    let mut arguments = serde_json::json!({
        "phaseSeed": null,
        "frozenChain": {"acceptance":false,"skeleton":false,"bodies":[]}
    });

    let prepared = prepare(
        &store,
        &run.id,
        &store.run_dir(&run.id),
        &mut arguments,
        &runtime,
        &criteria,
        &requirement_texts,
    )
    .unwrap();

    assert_eq!(prepared.unwrap().subjects, vec!["TASK-A-001"]);
    assert_eq!(
        arguments["forceBodySubjects"],
        serde_json::json!(["TASK-A-001"])
    );
    assert!(arguments["phaseSeed"].is_object());

    std::fs::write(
        store
            .run_dir(&run.id)
            .join("decomposition/reverify/remediation-reverify-report.json"),
        serde_json::json!({"report_id":"reverify-report", "subjects":["UNKNOWN-X"]}).to_string(),
    )
    .unwrap();
    let mut consumed_arguments = serde_json::json!({
        "phaseSeed": null,
        "forceBodySubjects": ["stale"],
        "frozenChain": {"acceptance":false,"skeleton":false,"bodies":[]}
    });
    let error = prepare(
        &store,
        &run.id,
        &store.run_dir(&run.id),
        &mut consumed_arguments,
        &runtime,
        &criteria,
        &requirement_texts,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("UNKNOWN-X"), "{error}");
    assert!(
        error.contains("remediation-reverify-report.json"),
        "{error}"
    );

    let ledger = Ledger {
        report_id: "reverify-report".into(),
        subjects: vec!["UNKNOWN-X".into()],
    };
    append_consumed(store.run_dir(&run.id).as_path(), &ledger).unwrap();
    let error = prepare(
        &store,
        &run.id,
        &store.run_dir(&run.id),
        &mut consumed_arguments,
        &runtime,
        &criteria,
        &requirement_texts,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("UNKNOWN-X"), "{error}");
    assert_eq!(
        consumed_arguments["forceBodySubjects"],
        serde_json::json!(["stale"])
    );
    assert!(consumed_arguments["phaseSeed"].is_null());

    let mut resumed = store.load_state(&run.id).unwrap();
    resumed.status = archon_workflow::RunStatus::Completed;
    store.save_state(&resumed).unwrap();
    assert_eq!(resumed.status, archon_workflow::RunStatus::Completed);
}

#[test]
fn forced_author_phase_advances_past_recorded_calls_without_rewriting_them() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path();
    let results = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"));
    for ordinal in [1, 3] {
        let record = archon_workflow::WorkflowV2CallRecord::new(
            "run",
            archon_workflow::WorkflowV2HostCall {
                id: format!("body-TASK-A-001-author-{ordinal}"),
                method: archon_workflow::WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: archon_workflow::WorkflowV2HostOptions::default(),
            },
            ordinal,
            format!("input-{ordinal}"),
            archon_workflow::WorkflowV2Result::accepted("answer"),
            Vec::new(),
        );
        results.save_call_record(&record).unwrap();
    }
    let prior_paths = [
        results.result_path("body-TASK-A-001-author-1"),
        results.result_path("body-TASK-A-001-author-3"),
    ];
    let prior_bytes = prior_paths
        .iter()
        .map(std::fs::read)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut arguments = serde_json::json!({
        "phaseSeed": {
            "author_ordinals": {"body-TASK-A-001": 0, "body-TASK-B-001": 4},
            "subjects": {}
        }
    });
    let ledger = Ledger {
        report_id: "reverify-report".into(),
        subjects: vec!["TASK-A-001".into()],
    };

    advance_forced_author_ordinals(run_dir, &mut arguments, &ledger).unwrap();

    assert_eq!(
        arguments["phaseSeed"]["author_ordinals"]["body-TASK-A-001"],
        3
    );
    assert_eq!(
        arguments["phaseSeed"]["author_ordinals"]["body-TASK-B-001"],
        4
    );
    assert_eq!(
        prior_paths
            .iter()
            .map(std::fs::read)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        prior_bytes
    );
}
