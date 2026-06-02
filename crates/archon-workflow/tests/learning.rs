use archon_workflow::{
    HeuristicWorkflowPlanner, Verification, WorkflowExecutor, WorkflowLearningSink,
    WorkflowPlanner, WorkflowPolicy, WorkflowStore, learning_records,
};

#[test]
fn accepted_artifacts_feed_durable_learning_records() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner
        .plan("Audit this repo with subagents")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    let summary = WorkflowLearningSink::new(store.clone())
        .record(&finished)
        .unwrap();
    assert_eq!(summary.records, finished.stages.len());
    assert!(summary.durable_records > 0);

    let durable = std::fs::read_to_string(
        store
            .run_dir(&finished.id)
            .join("learning")
            .join("durable-memory.jsonl"),
    )
    .unwrap();
    assert!(durable.contains("\"durable\":true"));
    assert!(!durable.contains("thinking"));
    assert!(summary.adapter_records >= summary.durable_records * 6);
}

#[test]
fn failed_and_forced_outputs_are_not_durable() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner.plan("Research a topic").unwrap();
    let mut run = store.create_run(spec).unwrap();
    let state = run.stage_mut("discover").unwrap();
    state.status = archon_workflow::StageStatus::Failed;
    state.error = Some("quality gate failed".into());

    let records = learning_records(&run);
    let plan = records
        .iter()
        .find(|record| record.stage_id == "discover")
        .unwrap();
    assert_eq!(plan.verification, Verification::Failed);
    assert!(!plan.durable);
}

#[test]
fn direct_learning_adapter_files_are_written() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner
        .plan("Research a topic with a final report")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);

    let learning_dir = store.run_dir(&run.id).join("learning");
    for file in [
        "adapter-sona.jsonl",
        "adapter-rlm.jsonl",
        "adapter-reflexion.jsonl",
        "adapter-reasoning-bank.jsonl",
        "adapter-jepa.jsonl",
        "adapter-world-model.jsonl",
    ] {
        let body = std::fs::read_to_string(learning_dir.join(file)).unwrap();
        assert!(
            body.contains("dynamic_workflow"),
            "{file} missing workflow trace"
        );
    }
}
