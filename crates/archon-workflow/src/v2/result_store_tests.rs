use super::super::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem, WorkflowV2WriteMode,
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
