//! The resume order: `prepare` advances a forced body subject's author
//! ordinal past its recorded calls, then the post-heal refresh rebuilds the
//! acceptance subject. The advance must survive the refresh, or the script
//! reuses a call id the records already hold.

use super::*;

fn call(
    run_id: &str,
    id: &str,
    method: archon_workflow::WorkflowV2HostMethod,
    result: archon_workflow::WorkflowV2Result,
) -> archon_workflow::WorkflowV2CallRecord {
    archon_workflow::WorkflowV2CallRecord::new(
        run_id,
        archon_workflow::WorkflowV2HostCall {
            id: id.into(),
            method,
            write_mode: None,
            options: Default::default(),
        },
        1,
        format!("input-{id}"),
        result,
        Vec::new(),
    )
}

#[test]
fn advanced_author_ordinals_survive_the_post_heal_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "ordinal-test".into(),
            task: "ordinal test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let run_dir = store.run_dir(&run.id);
    let results = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"));
    use archon_workflow::WorkflowV2HostMethod::{Agent, FinalReport};
    let report = |status, data| archon_workflow::WorkflowV2Result {
        status,
        data,
        ..Default::default()
    };
    for record in [
        call(
            &run.id,
            "final-report",
            FinalReport,
            report(
                archon_workflow::WorkflowV2Status::Accepted,
                serde_json::json!({"outcome": "accepted", "subjects": []}),
            ),
        ),
        call(
            &run.id,
            "reverify-report",
            FinalReport,
            report(
                archon_workflow::WorkflowV2Status::NeedsReview,
                serde_json::json!({"outcome": "needs_remediation", "supersedes_id": "final-report",
                                   "subjects": [{"task_id": "TASK-A-001"}]}),
            ),
        ),
        call(
            &run.id,
            "body-TASK-A-001-author-3",
            Agent,
            archon_workflow::WorkflowV2Result::accepted("body"),
        ),
    ] {
        results.save_call_record(&record).unwrap();
    }
    let event_log = archon_workflow::WorkflowEventLog::new(store.clone());
    for (kind, detail) in [
        (
            archon_workflow::WorkflowEventKind::DecompositionCompleted,
            serde_json::json!({"call_id": "final-report"}),
        ),
        (
            archon_workflow::WorkflowEventKind::DecompositionReverified,
            serde_json::json!({"report_id": "reverify-report", "supersedes_id": "final-report"}),
        ),
    ] {
        let seq = store.next_event_seq(&run.id).unwrap();
        event_log.emit(&run.id, seq, kind, detail).unwrap();
    }
    std::fs::create_dir_all(run_dir.join("decomposition/reverify")).unwrap();
    std::fs::write(
        run_dir.join("decomposition/reverify/remediation-reverify-report.json"),
        serde_json::json!({"report_id": "reverify-report", "subjects": ["TASK-A-001"]}).to_string(),
    )
    .unwrap();
    let tasks = tempfile::tempdir().unwrap();
    std::fs::write(
        tasks
            .path()
            .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        serde_json::json!({
            "schema_version": 1, "acceptance_digest": "digest",
            "tasks": [{"task_id": "TASK-A-001", "file_name": "TASK-A-001.md", "depends_on": [],
                       "blocks": [], "implements": [], "deliverable_contracts": []}]
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
    // The harness seed the resume hands the script, before the remediation.
    let mut seed = crate::command::workflow_decompose_seed::PhaseSeed {
        schema_version: 1,
        transition_index: 0,
        runtime: runtime.clone(),
        derived_at: "now".into(),
        author_ordinals: std::collections::BTreeMap::from([("body-TASK-B-001".into(), 4)]),
        pause_ordinals: Default::default(),
        subjects: std::collections::BTreeMap::from([(
            "acceptance".into(),
            crate::command::workflow_decompose_seed::SubjectSeed::Entries {
                gates: Vec::new(),
                candidate: None,
                replies: Vec::new(),
                unreadable_replies: Default::default(),
                carried_entries: Vec::new(),
                refuted_ids: Vec::new(),
                invalid: Default::default(),
                carried: 0,
            },
        )]),
        acceptance_refreshed_from_contract_digest: None,
        resume_frozen_chain: None,
        pause_generation_floor: 0,
        history_attempts: Default::default(),
        event_id: None,
    };
    let expected =
        serde_json::json!({"frozenChain": {"acceptance": false, "skeleton": false, "bodies": []}});
    let mut arguments =
        crate::command::workflow_decompose_seed::seeded_arguments(&expected, Some(&seed));
    let (criteria, texts) = (Default::default(), Default::default());
    let prepared = prepare(
        &store,
        &run.id,
        &run_dir,
        &mut arguments,
        &runtime,
        &criteria,
        &texts,
    )
    .unwrap();
    assert_eq!(prepared.unwrap().ledger.subjects, ["TASK-A-001"]);
    assert_eq!(
        arguments["phaseSeed"]["author_ordinals"]["body-TASK-A-001"],
        3
    );
    crate::command::workflow_decompose_seed::refresh_after_resume_heal(
        tasks.path(),
        &run_dir,
        &mut arguments,
        Some(&mut seed),
        &expected,
    )
    .unwrap();
    assert_eq!(
        arguments["phaseSeed"]["author_ordinals"]["body-TASK-A-001"], 3,
        "the refresh reset the advanced ordinal: {}",
        arguments["phaseSeed"]
    );
    assert_eq!(
        arguments["phaseSeed"]["author_ordinals"]["body-TASK-B-001"],
        4
    );
    assert!(arguments["phaseSeed"]["subjects"]["acceptance"].is_object());
}
