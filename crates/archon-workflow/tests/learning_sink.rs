//! The write half of the learning bridge: one stream, self-describing.

use archon_workflow::{
    StageStatus, Verification, WorkflowLearningSink, WorkflowSpec, WorkflowStore,
    read_learning_records,
};

fn spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: sink-test
task: Sink test
learning_hooks: [sona, world_model]
stages:
  - id: a
    kind: agent
    agent: tester
  - id: r
    kind: reduce
    depends_on: [a]
"#,
    )
    .unwrap()
}

fn seeded_store() -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let mut run = store.create_run(spec()).unwrap();
    let artifact = store
        .write_artifact(&run.id, "a", "input-hash", "md", b"body")
        .unwrap();
    let state = run.stage_mut("a").unwrap();
    state.status = StageStatus::Accepted;
    state.attempt = 2;
    state.quality_score = Some(0.75);
    state.artifacts.push(artifact);
    run.stage_mut("r").unwrap().status = StageStatus::Failed;
    store.save_state(&run).unwrap();
    let run_id = run.id.clone();
    (temp, store, run_id)
}

#[test]
fn sink_writes_exactly_one_record_stream() {
    let (_temp, store, run_id) = seeded_store();
    let run = store.load_state(&run_id).unwrap();
    let summary = WorkflowLearningSink::new(store.clone())
        .record(&run)
        .unwrap();
    assert_eq!(summary.records, 2);
    assert_eq!(summary.durable_records, 1);

    // The collapsed shape is the point: one file, not ten. The adapter fan-out
    // wrote a dozen copies of every outcome, demultiplexed by consumer before
    // any consumer existed.
    let learning_dir = store.run_dir(&run_id).join("learning");
    let files: Vec<String> = std::fs::read_dir(&learning_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(files, vec!["records.jsonl".to_string()]);
}

#[test]
fn records_carry_the_routing_selector_and_attribution() {
    let (_temp, store, run_id) = seeded_store();
    let run = store.load_state(&run_id).unwrap();
    WorkflowLearningSink::new(store.clone())
        .record(&run)
        .unwrap();

    let records = read_learning_records(&store, &run_id).unwrap();
    assert_eq!(records.len(), 2);

    let accepted = records.iter().find(|r| r.stage_id == "a").unwrap();
    assert_eq!(accepted.verification, Verification::Accepted);
    assert!(accepted.durable);
    // `learning_hooks` travels with the record: the reader routes from the
    // stream alone and needs no second file.
    assert_eq!(accepted.hooks, vec!["sona", "world_model"]);
    assert_eq!(accepted.agent.as_deref(), Some("tester"));
    assert_eq!(accepted.phase, "agent");
    assert_eq!(accepted.quality(), 0.75);
    assert_eq!(accepted.agent_key(), "tester");

    let failed = records.iter().find(|r| r.stage_id == "r").unwrap();
    assert_eq!(failed.verification, Verification::Failed);
    assert_eq!(failed.phase, "reduce");
    // No declared agent on the stage: fall back to the phase, which is what the
    // generated V2 path always produces.
    assert_eq!(failed.agent_key(), "reduce");
    assert_eq!(failed.quality(), 0.0);
}

#[test]
fn reading_a_run_that_never_recorded_is_not_an_error() {
    let (_temp, store, run_id) = seeded_store();
    assert!(read_learning_records(&store, &run_id).unwrap().is_empty());
}

#[test]
fn re_recording_replaces_rather_than_appends() {
    let (_temp, store, run_id) = seeded_store();
    let run = store.load_state(&run_id).unwrap();
    let sink = WorkflowLearningSink::new(store.clone());
    sink.record(&run).unwrap();
    sink.record(&run).unwrap();
    // A resume re-derives every stage's current state, so an appending write
    // would double-count the run's own stages.
    assert_eq!(read_learning_records(&store, &run_id).unwrap().len(), 2);
}
