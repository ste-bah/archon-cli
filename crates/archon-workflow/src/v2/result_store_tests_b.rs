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
