use archon_workflow::{
    HeuristicWorkflowPlanner, WorkflowApprovalDecision, WorkflowApprovalStore, WorkflowBundle,
    WorkflowExecutor, WorkflowPlanner, WorkflowPolicy, WorkflowStore,
};

#[test]
fn generated_run_writes_and_verifies_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner
        .plan("Audit workflow bundle behavior")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());

    let run = executor.start(spec).unwrap();
    let manifest = WorkflowBundle::verify(&store, &run.id).unwrap();

    assert_eq!(manifest.phase_count, run.spec.stages.len());
    assert!(store.run_dir(&run.id).join("workflow.js").exists());
    assert!(
        store
            .run_dir(&run.id)
            .join("workflow.compiled.yaml")
            .exists()
    );
}

#[test]
fn v2_write_mode_stages_are_listed_as_write_capable() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = archon_workflow::WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: v2-write-manifest
task: verify v2 write-capable manifest reporting
stages:
  - id: inventory
    kind: agent
    task: produce implementation items
    input:
      runtime: v2
      host_call: agent
  - id: implement
    kind: fanout
    task: coordinated implementation fanout
    depends_on: [inventory]
    input:
      runtime: v2
      host_call: fanout
      write_mode: coordinated
      source: inventory.items
"#,
    )
    .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());

    let run = executor.start(spec).unwrap();
    let manifest = WorkflowBundle::verify(&store, &run.id).unwrap();

    assert_eq!(manifest.write_capable_stages, vec!["implement"]);
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[test]
fn imported_spec_run_uses_wrapper_and_hash_verification_catches_tamper() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner
        .plan("Audit imported spec wrapper behavior")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());

    let run = executor.start_imported_spec(spec).unwrap();
    let harness = std::fs::read_to_string(store.run_dir(&run.id).join("workflow.js")).unwrap();
    assert!(harness.contains("runCompiledSpec"));
    WorkflowBundle::verify(&store, &run.id).unwrap();

    std::fs::write(store.run_dir(&run.id).join("workflow.js"), "tampered").unwrap();
    assert!(WorkflowBundle::verify(&store, &run.id).is_err());
}

#[test]
fn approval_store_records_run_once_decision_by_workflow_hash() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = HeuristicWorkflowPlanner
        .plan("Audit approval persistence behavior")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();

    let approvals = WorkflowApprovalStore::project(temp.path());
    let record = approvals
        .approve_run_once(temp.path(), &store, &run, "test")
        .unwrap();
    let inspection = approvals.inspect_run(temp.path(), &store, &run).unwrap();

    assert_eq!(record.decision, WorkflowApprovalDecision::RunOnce);
    assert_eq!(inspection.workflow_hash, record.workflow_hash);
    assert_eq!(inspection.decision.unwrap().decided_by, "test");
    assert!(approvals.path().exists());
}
