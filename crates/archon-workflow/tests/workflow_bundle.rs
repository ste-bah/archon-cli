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
fn approval_store_records_run_once_decision_by_approval_subject() {
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
    assert_eq!(inspection.compiled_hash, record.compiled_hash);
    assert_eq!(
        inspection.approval_subject_hash,
        record.approval_subject_hash
    );
    assert_eq!(inspection.decision.unwrap().decided_by, "test");
    assert!(approvals.path().exists());
}

#[test]
fn approval_store_does_not_reuse_denied_decision_when_compiled_hash_changes() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let first_spec = HeuristicWorkflowPlanner
        .plan("Audit first imported spec")
        .unwrap();
    let second_spec = HeuristicWorkflowPlanner
        .plan("Audit second imported spec with changed compiled metadata")
        .unwrap();
    let first = executor.start_imported_spec(first_spec).unwrap();
    let second = executor.start_imported_spec(second_spec).unwrap();
    let first_manifest = WorkflowBundle::verify(&store, &first.id).unwrap();
    let second_manifest = WorkflowBundle::verify(&store, &second.id).unwrap();
    assert_eq!(first_manifest.workflow_hash, second_manifest.workflow_hash);
    assert_ne!(first_manifest.compiled_hash, second_manifest.compiled_hash);

    let approvals = WorkflowApprovalStore::project(temp.path());
    let denied = approvals
        .deny_run(temp.path(), &store, &first, "test")
        .unwrap();
    assert_eq!(denied.decision, WorkflowApprovalDecision::Denied);

    let inspection = approvals.inspect_run(temp.path(), &store, &second).unwrap();
    assert!(inspection.decision.is_none());
    assert_ne!(
        inspection.approval_subject_hash,
        denied.approval_subject_hash
    );
}

#[test]
fn approval_store_reuses_approve_always_for_same_exact_subject() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let spec = HeuristicWorkflowPlanner
        .plan("Audit repeated imported spec approval")
        .unwrap();
    let first = executor.start_imported_spec(spec.clone()).unwrap();
    let second = executor.start_imported_spec(spec).unwrap();

    let approvals = WorkflowApprovalStore::project(temp.path());
    let approved = approvals
        .approve_always_for_project(temp.path(), &store, &first, "test")
        .unwrap();
    let inspection = approvals.inspect_run(temp.path(), &store, &second).unwrap();
    let decision = inspection.decision.unwrap();

    assert_eq!(
        approved.decision,
        WorkflowApprovalDecision::AlwaysForProject
    );
    assert_eq!(
        inspection.approval_subject_hash,
        approved.approval_subject_hash
    );
    assert_eq!(
        decision.decision,
        WorkflowApprovalDecision::AlwaysForProject
    );
}

#[test]
fn approval_store_does_not_reuse_run_once_for_same_subject_different_run() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let spec = HeuristicWorkflowPlanner
        .plan("Audit run once imported spec approval")
        .unwrap();
    let first = executor.start_imported_spec(spec.clone()).unwrap();
    let second = executor.start_imported_spec(spec).unwrap();

    let approvals = WorkflowApprovalStore::project(temp.path());
    approvals
        .approve_run_once(temp.path(), &store, &first, "test")
        .unwrap();
    let inspection = approvals.inspect_run(temp.path(), &store, &second).unwrap();

    assert!(inspection.decision.is_none());
}

#[test]
fn approval_store_ignores_legacy_hash_only_records_for_auto_gate() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = HeuristicWorkflowPlanner
        .plan("Audit legacy approval record behavior")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start_imported_spec(spec).unwrap();
    let manifest = WorkflowBundle::verify(&store, &run.id).unwrap();
    let approvals = WorkflowApprovalStore::project(temp.path());
    let parent = approvals.path().parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    let project_root = temp
        .path()
        .canonicalize()
        .unwrap_or_else(|_| temp.path().to_path_buf())
        .display()
        .to_string();
    std::fs::write(
        approvals.path(),
        serde_json::to_vec_pretty(&serde_json::json!({
            "records": [{
                "workflow_hash": manifest.workflow_hash,
                "project_root": project_root,
                "workflow_name": manifest.name.clone(),
                "decision": "denied",
                "decided_at": "2026-01-01T00:00:00Z",
                "decided_by": "legacy-test",
                "run_id": run.id.clone(),
                "phase_count": manifest.phase_count,
                "max_agents": manifest.max_agents,
                "max_parallelism": manifest.max_parallelism,
                "write_capable_stages": manifest.write_capable_stages.clone(),
                "external_requirements": [],
                "raw_script_path": store.run_dir(&run.id).join("workflow.js").display().to_string(),
                "compiled_spec_path": store.run_dir(&run.id).join("workflow.compiled.yaml").display().to_string(),
                "origin": "imported_spec_wrapper"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let inspection = approvals.inspect_run(temp.path(), &store, &run).unwrap();
    assert!(inspection.decision.is_none());
}

#[test]
fn approval_subject_includes_generated_metadata_hash_when_present() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = HeuristicWorkflowPlanner
        .plan("Audit generated metadata approval subject")
        .unwrap();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start_imported_spec(spec).unwrap();
    let metadata_path = store.run_dir(&run.id).join("v2/generated-metadata.json");
    std::fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
    std::fs::write(&metadata_path, br#"{"host_call_manifest":[]}"#).unwrap();

    let approvals = WorkflowApprovalStore::project(temp.path());
    let record = approvals
        .approve_always_for_project(temp.path(), &store, &run, "test")
        .unwrap();

    assert!(record.generated_metadata_hash.is_some());
    assert!(!record.approval_subject_hash.is_empty());
}
