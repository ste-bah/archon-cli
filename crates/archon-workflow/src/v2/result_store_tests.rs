use super::super::{
    WorkflowV2CallExecution, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind, WorkflowV2WriteMode,
};
use super::*;

fn call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    }
}

#[test]
fn call_records_are_sanitized_before_persistence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let mut result = WorkflowV2Result::accepted("done with token=supersecret");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Other,
        "authorization: bearer-secret should not persist",
    ));
    result.data = serde_json::json!({
        "raw_text": "private provider payload",
        "nested": { "api_key": "secret", "safe": "visible" }
    });
    let record = WorkflowV2CallRecord::new(
        "wf-test",
        call("discover"),
        0,
        "input".to_string(),
        result,
        Vec::new(),
    );

    store.save_call_record(&record).expect("save record");
    let raw = std::fs::read_to_string(store.result_path("discover")).expect("persisted record");

    assert!(!raw.contains("supersecret"));
    assert!(!raw.contains("authorization"));
    assert!(!raw.contains("raw_text"));
    assert!(!raw.contains("api_key"));
    assert!(raw.contains("visible"));
}

#[test]
fn branch_outcomes_can_be_loaded_across_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let first = WorkflowV2BranchOutcome {
        item_id: "first-item".to_string(),
        role: "coder".to_string(),
        status: WorkflowV2Status::Noop,
        result: None,
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    };
    let second = WorkflowV2BranchOutcome {
        item_id: "second-item".to_string(),
        status: WorkflowV2Status::Accepted,
        ..first.clone()
    };
    store
        .save_branch_outcome("implementation-wave-1", &first)
        .expect("first outcome");
    store
        .save_branch_outcome("verification-wave-1", &second)
        .expect("second outcome");

    let outcomes = store.load_branch_outcomes().expect("load outcomes");

    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].item_id, "first-item");
    assert_eq!(outcomes[1].item_id, "second-item");
}

#[test]
fn branch_outcomes_are_sanitized_before_persistence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let mut result = WorkflowV2Result::accepted("done");
    result.data = serde_json::json!({ "access_token": "do-not-store", "safe": "ok" });
    let outcome = WorkflowV2BranchOutcome {
        item_id: "item-1".to_string(),
        role: "coder".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: Some("token=branchsecret".to_string()),
        failure_kind: None,
        item_input_hash: Some("test-input-hash-item-1".to_string()),
        completion_evidence: Vec::new(),
    };

    let path = store
        .save_branch_outcome("implementation", &outcome)
        .expect("save branch");
    let raw = std::fs::read_to_string(path).expect("persisted branch");

    assert!(!raw.contains("do-not-store"));
    assert!(!raw.contains("access_token"));
    assert!(!raw.contains("branchsecret"));
    assert!(raw.contains("ok"));
}

#[test]
fn scaffold_hash_participates_in_source_reuse() {
    let mut result = WorkflowV2Result::accepted("done");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Other,
        "scaffold reuse proof",
    ));
    let record = WorkflowV2CallRecord::new(
        "wf-test",
        call("implementation-wave-1"),
        1,
        "input-hash".to_string(),
        result,
        Vec::new(),
    )
    .with_source_metadata(Some("source-a".to_string()), None)
    .with_scaffold_hash(Some("scaffold-a".to_string()));

    assert!(record.is_reusable_for_source_and_scaffold(
        "input-hash",
        Some("source-a"),
        Some("scaffold-a")
    ));
    assert!(!record.is_reusable_for_source_and_scaffold(
        "input-hash",
        Some("source-a"),
        Some("scaffold-b")
    ));
    assert!(!record.is_reusable_for_source_and_scaffold("input-hash", Some("source-a"), None));
}

#[test]
fn dynamic_wave_restart_invalidates_dependent_source_graph_closure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    save_wave(
        &store,
        "implementation-wave-1",
        "TASK-TDL-001",
        [],
        ["TASK-TDL-001"],
    );
    save_wave(
        &store,
        "implementation-wave-2",
        "TASK-TDL-010",
        ["TASK-TDL-001"],
        ["TASK-TDL-010"],
    );
    save_wave(
        &store,
        "implementation-wave-3",
        "TASK-TDL-020",
        ["TASK-TDL-010"],
        ["TASK-TDL-020"],
    );
    store
        .save_checkpoint(&WorkflowV2Checkpoint {
            completed_call_ids: vec![
                "implementation-wave-1".to_string(),
                "implementation-wave-2".to_string(),
                "implementation-wave-3".to_string(),
            ],
        })
        .expect("checkpoint");

    let invalidated = store
        .invalidate_dynamic_wave_dependents("implementation-wave-1")
        .expect("invalidate");

    assert_eq!(
        invalidated,
        vec![
            "implementation-wave-1".to_string(),
            "implementation-wave-2".to_string(),
            "implementation-wave-3".to_string()
        ]
    );
    assert_eq!(
        store
            .load_call_record("implementation-wave-2")
            .unwrap()
            .unwrap()
            .invalidated_by
            .as_deref(),
        Some("implementation-wave-1")
    );
    assert_eq!(
        store.load_checkpoint().unwrap().unwrap().completed_call_ids,
        Vec::<String>::new()
    );
}

#[test]
fn dynamic_wave_restart_invalidates_generated_downstream_non_implementation_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    save_wave(
        &store,
        "implementation-wave-1",
        "TASK-TDL-001",
        [],
        ["TASK-TDL-001"],
    );
    save_wave(
        &store,
        "verification-wave-1",
        "TASK-TDL-001",
        [],
        ["TASK-TDL-001"],
    );
    save_wave(
        &store,
        "final-acceptance-report",
        "TASK-TDL-001",
        [],
        ["TASK-TDL-001"],
    );
    store
        .save_checkpoint(&WorkflowV2Checkpoint {
            completed_call_ids: vec![
                "implementation-wave-1".to_string(),
                "verification-wave-1".to_string(),
                "final-acceptance-report".to_string(),
            ],
        })
        .expect("checkpoint");

    let invalidated = store
        .invalidate_dynamic_wave_dependents("implementation-wave-1")
        .expect("invalidate");

    assert!(invalidated.contains(&"implementation-wave-1".to_string()));
    assert!(invalidated.contains(&"verification-wave-1".to_string()));
    assert!(invalidated.contains(&"final-acceptance-report".to_string()));
    assert_eq!(
        store.load_checkpoint().unwrap().unwrap().completed_call_ids,
        Vec::<String>::new()
    );
}

fn execution<const D: usize>(id: &str, depends_on: [&str; D]) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: call(id),
        input: serde_json::Value::Null,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
    }
}

fn implementation_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".to_string()),
            target_files_from_item: true,
            ..WorkflowV2HostOptions::default()
        },
    }
}

fn save_task_record<const T: usize, const D: usize>(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    task_ids: [&str; T],
    dependency_ids: [&str; D],
) {
    let task_ids = task_ids.into_iter().map(str::to_string).collect::<Vec<_>>();
    let graph = WorkflowV2SourceTaskGraph::new(
        vec![
            "TASK-ALPHA-010".to_string(),
            "TASK-ALPHA-020".to_string(),
            "TASK-ALPHA-030".to_string(),
        ],
        vec![WorkflowV2SourceTaskItem {
            item_id: format!("item-{call_id}"),
            canonical_task_ids: task_ids.clone(),
            dependency_ids: dependency_ids.into_iter().map(str::to_string).collect(),
            target_files: Vec::new(),
            declared_target_files: Vec::new(),
            target_file_expansions: Vec::new(),
            acceptance_criteria: Vec::new(),
            focused_verification: Vec::new(),
            expected_evidence: Vec::new(),
            artifact_requirements: Vec::new(),
            required_tools: Vec::new(),
        }],
        task_ids,
    );
    let record = WorkflowV2CallRecord::new(
        "wf-test",
        implementation_call(call_id),
        1,
        format!("hash-{call_id}"),
        WorkflowV2Result::accepted(format!("{call_id} accepted")),
        Vec::new(),
    )
    .with_source_metadata(Some(format!("source-{call_id}")), Some(graph));
    store.save_call_record(&record).expect("save task record");
}

fn save_task_outcome(store: &WorkflowV2ResultStore, call_id: &str, item_id: &str, task_id: &str) {
    let evidence = WorkflowV2TaskCompletionEvidence::new(
        task_id,
        WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
        call_id,
        item_id,
        WorkflowV2Status::Accepted,
    );
    store
        .save_branch_outcome(
            call_id,
            &WorkflowV2BranchOutcome {
                item_id: item_id.to_string(),
                role: "coder".to_string(),
                status: WorkflowV2Status::Accepted,
                result: Some(WorkflowV2Result::accepted(format!("{task_id} accepted"))),
                error: None,
                failure_kind: None,
                item_input_hash: Some(format!("hash-{item_id}")),
                completion_evidence: vec![evidence],
            },
        )
        .expect("save task outcome");
}

fn save_wave<const D: usize, const C: usize>(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    task_id: &str,
    dependency_ids: [&str; D],
    completed_ids: [&str; C],
) {
    let universe = ["TASK-TDL-001", "TASK-TDL-010", "TASK-TDL-020"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let graph = WorkflowV2SourceTaskGraph::new(
        universe,
        vec![WorkflowV2SourceTaskItem {
            item_id: task_id.to_string(),
            canonical_task_ids: vec![task_id.to_string()],
            dependency_ids: dependency_ids.into_iter().map(str::to_string).collect(),
            target_files: Vec::new(),
            declared_target_files: Vec::new(),
            target_file_expansions: Vec::new(),
            acceptance_criteria: Vec::new(),
            focused_verification: Vec::new(),
            expected_evidence: Vec::new(),
            artifact_requirements: Vec::new(),
            required_tools: Vec::new(),
        }],
        completed_ids.into_iter().map(str::to_string).collect(),
    );
    let record = WorkflowV2CallRecord::new(
        "wf-test",
        implementation_call(call_id),
        1,
        format!("hash-{call_id}"),
        WorkflowV2Result::accepted(format!("{call_id} accepted")),
        Vec::new(),
    )
    .with_source_metadata(Some(format!("source-{call_id}")), Some(graph));
    store.save_call_record(&record).expect("save wave");
}

#[test]
fn task_restart_invalidation_uses_canonical_task_graph_and_preserves_unrelated_outcomes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let executions = vec![
        execution("alpha-write", []),
        execution("beta-check", ["alpha-write"]),
        execution("gamma-write", []),
        execution("terminal-summary", ["beta-check", "gamma-write"]),
    ];
    save_task_record(&store, "alpha-write", ["TASK-ALPHA-010"], []);
    save_task_record(&store, "beta-check", ["TASK-ALPHA-020"], ["TASK-ALPHA-010"]);
    save_task_record(&store, "gamma-write", ["TASK-ALPHA-030"], []);
    save_task_record(&store, "terminal-summary", ["TASK-ALPHA-020"], []);
    save_task_outcome(&store, "alpha-write", "item-alpha", "TASK-ALPHA-010");
    save_task_outcome(&store, "beta-check", "item-beta", "TASK-ALPHA-020");
    save_task_outcome(&store, "gamma-write", "item-gamma", "TASK-ALPHA-030");
    store
        .save_checkpoint(&WorkflowV2Checkpoint {
            completed_call_ids: vec![
                "alpha-write".to_string(),
                "beta-check".to_string(),
                "gamma-write".to_string(),
                "terminal-summary".to_string(),
            ],
        })
        .expect("checkpoint");

    let invalidation = store
        .invalidate_task_and_dependents(
            &executions,
            "TASK-ALPHA-010",
            &["TASK-ALPHA-010", "TASK-ALPHA-020"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            "restart-task:TASK-ALPHA-010",
        )
        .expect("invalidate task");

    assert_eq!(
        invalidation.invalidated_call_ids,
        vec![
            "alpha-write".to_string(),
            "beta-check".to_string(),
            "terminal-summary".to_string(),
        ]
    );
    assert_eq!(invalidation.deleted_branch_outcomes.len(), 2);
    assert!(
        store
            .load_branch_outcome("alpha-write", "item-alpha")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .load_branch_outcome("beta-check", "item-beta")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .load_branch_outcome("gamma-write", "item-gamma")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store.load_checkpoint().unwrap().unwrap().completed_call_ids,
        vec!["gamma-write".to_string()]
    );
    assert_eq!(
        store
            .load_call_record("alpha-write")
            .unwrap()
            .unwrap()
            .invalidated_by
            .as_deref(),
        Some("restart-task:TASK-ALPHA-010")
    );
}

#[test]
fn superseding_execution_archives_prior_call_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let first = WorkflowV2CallRecord::new(
        "wf-test",
        call("implementation-wave-1"),
        0,
        "input-original".to_string(),
        WorkflowV2Result::accepted("original cycle"),
        Vec::new(),
    );
    store.save_call_record(&first).expect("save first");

    // Same-execution re-save (e.g. an invalidation stamp) keeps the slot.
    let mut stamped = first.clone();
    stamped.invalidated_by = Some("upstream".to_string());
    store
        .save_call_record(&stamped)
        .expect("re-save same execution");
    let superseded_dir = temp.path().join("results").join("superseded");
    assert!(!superseded_dir.exists(), "in-place update must not archive");

    // A rerouted cycle reusing the id is a NEW execution: prior record archives.
    let rerouted = WorkflowV2CallRecord::new(
        "wf-test",
        call("implementation-wave-1"),
        0,
        "input-reroute".to_string(),
        WorkflowV2Result::accepted("rerouted cycle"),
        Vec::new(),
    );
    store.save_call_record(&rerouted).expect("save rerouted");

    let latest = store
        .load_call_record("implementation-wave-1")
        .expect("load")
        .expect("record");
    assert_eq!(latest.input_hash, "input-reroute");
    let archived: Vec<_> = std::fs::read_dir(&superseded_dir)
        .expect("superseded dir")
        .flatten()
        .collect();
    assert_eq!(archived.len(), 1, "original record must be archived");
    let raw = std::fs::read_to_string(archived[0].path()).expect("archived record");
    assert!(raw.contains("input-original"));
    // The archive directory must not pollute load_call_records.
    let all = store.load_call_records().expect("load all");
    assert_eq!(all.len(), 1);
}

#[test]
fn rapid_superseding_executions_preserve_every_prior_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    for cycle in 0..5 {
        let record = WorkflowV2CallRecord::new(
            "wf-test",
            call("implementation-wave-rapid"),
            0,
            format!("input-{cycle}"),
            WorkflowV2Result::accepted(format!("cycle {cycle}")),
            Vec::new(),
        );
        store.save_call_record(&record).expect("save cycle");
    }

    let superseded_dir = temp.path().join("results").join("superseded");
    assert_eq!(
        std::fs::read_dir(superseded_dir).expect("archive").count(),
        4,
        "every displaced record must retain a unique archive slot"
    );
}

#[test]
fn superseding_execution_archives_prior_branch_outcome() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let outcome = WorkflowV2BranchOutcome {
        item_id: "impl-item-1".to_string(),
        role: "coder".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(WorkflowV2Result::accepted("original")),
        error: None,
        failure_kind: None,
        item_input_hash: Some("hash-a".to_string()),
        completion_evidence: Vec::new(),
    };
    store
        .save_branch_outcome("implementation-wave-1", &outcome)
        .expect("save first");

    // Same-hash re-save is an in-place update.
    store
        .save_branch_outcome("implementation-wave-1", &outcome)
        .expect("re-save");
    let superseded_dir = temp
        .path()
        .join("branches")
        .join("implementation-wave-1")
        .join("superseded");
    assert!(!superseded_dir.exists());

    let mut rerouted = outcome.clone();
    rerouted.item_input_hash = Some("hash-b".to_string());
    rerouted.result = Some(WorkflowV2Result::accepted("rerouted"));
    store
        .save_branch_outcome("implementation-wave-1", &rerouted)
        .expect("save rerouted");
    assert_eq!(
        std::fs::read_dir(&superseded_dir).expect("dir").count(),
        1,
        "prior outcome must be archived"
    );
    // Archived subdirectory must not pollute load_branch_outcomes.
    let outcomes = store.load_branch_outcomes().expect("load outcomes");
    assert_eq!(outcomes.len(), 1);
}
