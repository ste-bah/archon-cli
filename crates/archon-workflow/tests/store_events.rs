use archon_workflow::{
    WorkflowEventKind, WorkflowEventLog, WorkflowSpec, WorkflowStore, contains_forbidden_field,
};
use serde_json::json;

fn spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: store-test
task: Store test
stages:
  - id: a
    kind: agent
    agent: tester
  - id: r
    kind: reduce
    reducer: evidence_weighted_report
    depends_on: [a]
"#,
    )
    .unwrap()
}

#[test]
fn run_dir_layout_created_and_listed() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let run = store.create_run(spec()).unwrap();
    let dir = store.run_dir(&run.id);
    for path in [
        "manifest.toml",
        "spec.yaml",
        "state.json",
        "events.jsonl",
        "artifacts",
        "agent-outputs",
        "prompts",
        "reducers",
        "quality",
        "learning",
    ] {
        assert!(dir.join(path).exists(), "missing {path}");
    }
    assert_eq!(store.list_runs().unwrap().len(), 1);
}

#[test]
fn artifact_hash_stable_and_reuse_rejects_hash_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let mut run = store.create_run(spec()).unwrap();
    let mut artifact = store
        .write_artifact(&run.id, "a", "input-hash", "txt", b"hello")
        .unwrap();
    artifact.accepted = true;
    let state = run.stage_mut("a").unwrap();
    state.status = archon_workflow::StageStatus::Accepted;
    state.artifacts.push(artifact.clone());
    store.save_state(&run).unwrap();
    store
        .validate_for_reuse(&run, &artifact, "input-hash")
        .unwrap();
    std::fs::write(store.run_dir(&run.id).join(&artifact.path), b"tampered").unwrap();
    assert!(
        store
            .validate_for_reuse(&run, &artifact, "input-hash")
            .is_err()
    );
}

#[test]
fn event_sanitize_strips_provider_private_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let run = store.create_run(spec()).unwrap();
    let log = WorkflowEventLog::new(store.clone());
    let event = log
        .emit(
            &run.id,
            1,
            WorkflowEventKind::StageStarted,
            json!({
                "stage": "a",
                "thinking": "secret",
                "nested": {"encrypted_reasoning": "secret", "status": "ok"},
                "authorization": "Bearer bad"
            }),
        )
        .unwrap();
    assert!(!contains_forbidden_field(&event.detail));
    let raw = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(!raw.contains("secret"));
}
