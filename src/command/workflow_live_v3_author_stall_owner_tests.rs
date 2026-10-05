//! Issue 324 round 2: an author stall pause by a generation that no longer
//! owns the run (#291) changes no hand-over marker before it is refused.
use super::*;

fn paused_run() -> (tempfile::TempDir, WorkflowStore, String, u64) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "author-stall-owner".to_string(),
        task: "test".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let run_id = store.create_run(spec).expect("run").id;
    let stale = store.load_state(&run_id).unwrap().generation;
    // An operator pause moves the generation on: `stale` owns nothing now.
    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    let marker = serde_json::json!({ "attempt": 3 });
    let path = format!("rejected-scripts/{STALL_MARKER}");
    store.write_run_json(&run_id, path, &marker).unwrap();
    (temp, store, run_id, stale)
}

fn marker(store: &WorkflowStore, run_id: &str) -> Option<String> {
    let path = store
        .run_dir(run_id)
        .join("rejected-scripts")
        .join(STALL_MARKER);
    std::fs::read_to_string(path).ok()
}

#[test]
fn a_stale_generation_neither_writes_nor_removes_the_marker() {
    for held in [Some(6), None] {
        let (_temp, store, run_id, stale) = paused_run();
        let before = marker(&store, &run_id);
        let stall = AuthorStall::Defects {
            reason: "stale finding".to_string(),
        };
        let attempts = StallAttempts {
            defects: 6,
            transports: 0,
            recorded: 6,
            held,
        };
        let refused = pause_on_author_stall(&store, &run_id, stale, stall, attempts);
        assert!(
            !refused.to_string().contains("paused, not failed"),
            "{refused}"
        );
        assert_eq!(marker(&store, &run_id), before, "held {held:?}");
        let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap_or_default();
        assert!(!events.contains("author_stall_pause"));
    }
}
