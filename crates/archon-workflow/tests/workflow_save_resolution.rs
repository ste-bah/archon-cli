use archon_workflow::{
    HeuristicWorkflowPlanner, WorkflowCommandRegistry, WorkflowExecutor, WorkflowPlanner,
    WorkflowPolicy, WorkflowStore,
};

#[test]
fn save_command_preserves_harness_and_compiled_spec_without_stale_run_id() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = HeuristicWorkflowPlanner
        .plan("Audit saved workflow command behavior")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();

    let registry = WorkflowCommandRegistry::project(temp.path());
    let saved = registry.save_run("repo-audit", &store, &run).unwrap();
    let loaded = registry.load("repo-audit").unwrap().unwrap();

    assert_eq!(saved.name, "repo-audit");
    assert_eq!(loaded.spec.name, saved.spec.name);
    assert_eq!(loaded.harness_source, saved.harness_source);
    assert_eq!(loaded.manifest.id, "command:repo-audit");
    assert!(!loaded.manifest.id.contains(&run.id));
    assert!(loaded.command_dir.join("workflow.js").exists());
    assert!(loaded.command_dir.join("workflow.compiled.yaml").exists());
}
