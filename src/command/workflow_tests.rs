use super::{
    GeneratedV2RestartTarget, WorkflowHandler, cli_action, generated_v2_restart_target,
    invalidate_generated_v2_call, invalidate_generated_v2_item, restart_task_workflow,
    stage_id_for_task, status_text,
};
use crate::cli_args::WorkflowAction;
use crate::command::registry::CommandHandler;
use crate::command::test_support::{CtxBuilder, drain_tui_events};
use archon_tui::app::TuiEvent;
use archon_workflow::run::StageState;
use archon_workflow::{
    CommandAction, ProviderTier, RetryPolicy, RunStatus, StageKind, StageSpec, StageStatus,
    WorkflowBundle, WorkflowBundleOrigin, WorkflowRun, WorkflowSpec, WorkflowStore,
    WorkflowV2BranchOutcome, WorkflowV2CallRecord, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2WriteMode,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn workflow_list_completes_tui_slash_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let (mut ctx, mut rx) = CtxBuilder::new()
        .with_working_dir(temp.path().to_path_buf())
        .build();

    WorkflowHandler
        .execute(&mut ctx, &[String::from("list")])
        .unwrap();

    let events = drain_tui_events(&mut rx);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TuiEvent::OpenViewRows { .. })),
        "workflow list should emit workflow rows"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TuiEvent::SlashCommandComplete)),
        "workflow command must complete the slash lifecycle"
    );
}

#[test]
fn run_resume_from_uses_existing_v2_resume_path() {
    let action = WorkflowAction::Run {
        spec_file: None,
        from_template: None,
        resume_from: Some("prior-run".to_string()),
        live: true,
        yes: true,
        task: vec!["same canary task".to_string()],
    };

    let (command, _) = cli_action(&action).expect("resume-from action");

    assert!(matches!(
        command,
        CommandAction::Resume { run_id } if run_id == "prior-run"
    ));
}

#[test]
fn restart_task_resolver_matches_canonical_task_variants() {
    let run = WorkflowRun::new(test_spec(), tempfile::tempdir().unwrap().path());

    assert_eq!(
        stage_id_for_task(&run, "T010").as_deref(),
        Some("implement-T010-T020")
    );
    assert_eq!(
        stage_id_for_task(&run, "TASK-GEN-010").as_deref(),
        Some("implement-T010-T020")
    );
}

#[test]
fn restart_task_command_rewinds_matching_stage() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let mut run = store.create_run(test_spec()).unwrap();
    run.status = RunStatus::Failed;
    run.stages.get_mut("implement-T010-T020").unwrap().status = StageStatus::Failed;
    run.stages.get_mut("implement-T010-T020").unwrap().error = Some("boom".to_string());
    store.save_state(&run).unwrap();

    let output = restart_task_workflow(&store, &run.id, "TASK-GEN-010").unwrap();
    let reloaded = store.load_state(&run.id).unwrap();

    assert!(output.contains("task TASK-GEN-010 mapped to stage implement-T010-T020"));
    assert_eq!(reloaded.status, RunStatus::Running);
    assert_eq!(
        reloaded.stages.get("implement-T010-T020").unwrap().status,
        StageStatus::Pending
    );
    assert!(
        reloaded
            .stages
            .get("implement-T010-T020")
            .unwrap()
            .error
            .is_none()
    );
}

#[test]
fn status_summary_reports_current_stage_and_next_action() {
    let mut run = WorkflowRun::new(test_spec(), tempfile::tempdir().unwrap().path());
    run.status = RunStatus::Failed;
    run.stages.get_mut("implement-T010-T020").unwrap().status = StageStatus::Failed;
    run.stages.get_mut("implement-T010-T020").unwrap().error =
        Some("verification failed".to_string());

    let summary = status_text(&run);

    assert!(summary.contains("current=implement-T010-T020"));
    assert!(summary.contains("error=verification failed"));
    assert!(summary.contains(&format!("next=/workflow repair {}", run.id)));
}

#[test]
fn generated_v2_restart_item_invalidates_parent_fanout_call() {
    let action = archon_workflow::LifecycleAction::RestartItem {
        stage_id: "implementation-fanout".to_string(),
        item_id: "implementation-fanout-T010".to_string(),
    };

    assert_eq!(
        generated_v2_restart_target(&action),
        Some(GeneratedV2RestartTarget::Item {
            call_id: "implementation-fanout".to_string(),
            item_id: "implementation-fanout-T010".to_string(),
        })
    );
}

#[test]
fn generated_v2_restart_item_deletes_only_requested_branch_cache() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(test_spec()).unwrap();
    let harness = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return typed items." });
  const implementation = await w.fanout("implementation", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "worktree", task: "Implement one item." });
  await w.finalReport("final", { inputs: [inventory, implementation], task: "Report evidence." });
}
"#;
    WorkflowBundle::create_for_run(
        &store,
        &run,
        harness,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    save_test_branch(&v2_store, "implementation", "implementation-T001");
    save_test_branch(&v2_store, "implementation", "implementation-T002");

    let invalidated = invalidate_generated_v2_item(&store, &run, "implementation", "T001").unwrap();

    assert!(invalidated.iter().any(|id| id == "implementation"));
    assert!(
        v2_store
            .load_branch_outcome("implementation", "implementation-T001")
            .unwrap()
            .is_none()
    );
    assert!(
        v2_store
            .load_branch_outcome("implementation", "implementation-T002")
            .unwrap()
            .is_some()
    );
}

#[test]
fn generated_v2_restart_stage_deletes_all_branch_cache_for_call() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(test_spec()).unwrap();
    let harness = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return typed items." });
  const implementation = await w.fanout("implementation", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "worktree", task: "Implement one item." });
  await w.finalReport("final", { inputs: [inventory, implementation], task: "Report evidence." });
}
"#;
    WorkflowBundle::create_for_run(
        &store,
        &run,
        harness,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    save_test_branch(&v2_store, "implementation", "implementation-T001");
    save_test_branch(&v2_store, "implementation", "implementation-T002");

    let invalidated = invalidate_generated_v2_call(&store, &run, "implementation").unwrap();

    assert!(invalidated.iter().any(|id| id == "implementation"));
    assert!(
        invalidated
            .iter()
            .any(|id| id == "implementation:branches(2)")
    );
    assert!(
        v2_store
            .load_branch_outcome("implementation", "implementation-T001")
            .unwrap()
            .is_none()
    );
    assert!(
        v2_store
            .load_branch_outcome("implementation", "implementation-T002")
            .unwrap()
            .is_none()
    );
}

#[test]
fn generated_v2_restart_stage_invalidates_downstream_script_consumers() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let mut run = store.create_run(test_spec()).unwrap();
    let harness = r#"
export default async function workflow(w) {
  const discoveryItems = [{ id: "prd" }, { id: "code" }];
  const discovery = await w.parallel("readonly-discovery", discoveryItems, { role: "researcher", task: "Read source evidence." });
  const inventory = await w.reduce("dependency-aware-implementation-inventory", [discovery], { role: "reducer", task: "Create typed implementation inventory." });
  await w.fanout("implementation-wave-1", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "coordinated", task: "Implement one item." });
}
"#;
    WorkflowBundle::create_for_run(
        &store,
        &run,
        harness,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    for stage_id in [
        "readonly-discovery",
        "dependency-aware-implementation-inventory",
        "implementation-wave-1",
    ] {
        run.stages
            .insert(stage_id.to_string(), running_stage(stage_id));
    }
    store.save_state(&run).unwrap();

    // Restart invalidation reads the host-call manifest persisted with the
    // run (as approval-time planning writes it), not the script source.
    let manifest_calls = vec![
        test_v2_call("readonly-discovery", WorkflowV2HostMethod::Parallel, None),
        test_v2_call(
            "dependency-aware-implementation-inventory",
            WorkflowV2HostMethod::Reduce,
            Some("[readonly-discovery]"),
        ),
        test_v2_call(
            "implementation-wave-1",
            WorkflowV2HostMethod::Fanout,
            Some("dependency-aware-implementation-inventory.items"),
        ),
    ];
    std::fs::create_dir_all(store.run_dir(&run.id).join("v2")).unwrap();
    std::fs::write(
        store.run_dir(&run.id).join("v2/generated-metadata.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "workflow-generated-v2-metadata-v1",
            "generated_scaffold": {
                "host_call_manifest": manifest_calls,
            },
        }))
        .unwrap(),
    )
    .unwrap();

    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    save_test_call_record(
        &v2_store,
        &run.id,
        test_v2_call("readonly-discovery", WorkflowV2HostMethod::Parallel, None),
        vec![],
    );
    save_test_call_record(
        &v2_store,
        &run.id,
        test_v2_call(
            "dependency-aware-implementation-inventory",
            WorkflowV2HostMethod::Reduce,
            Some("[readonly-discovery]"),
        ),
        vec!["readonly-discovery".to_string()],
    );
    save_test_call_record(
        &v2_store,
        &run.id,
        test_v2_call(
            "implementation-wave-1",
            WorkflowV2HostMethod::Fanout,
            Some("dependency-aware-implementation-inventory.items"),
        ),
        vec!["dependency-aware-implementation-inventory".to_string()],
    );

    let invalidated = invalidate_generated_v2_call(&store, &run, "readonly-discovery").unwrap();

    assert_eq!(
        invalidated
            .iter()
            .filter(|id| !id.contains(':'))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "dependency-aware-implementation-inventory".to_string(),
            "implementation-wave-1".to_string(),
            "readonly-discovery".to_string(),
        ]
    );
    assert_eq!(
        v2_store
            .load_call_record("dependency-aware-implementation-inventory")
            .unwrap()
            .unwrap()
            .invalidated_by,
        Some("readonly-discovery".to_string())
    );
    assert_eq!(
        v2_store
            .load_call_record("implementation-wave-1")
            .unwrap()
            .unwrap()
            .invalidated_by,
        Some("readonly-discovery".to_string())
    );
    let reloaded = store.load_state(&run.id).unwrap();
    for stage_id in [
        "readonly-discovery",
        "dependency-aware-implementation-inventory",
        "implementation-wave-1",
    ] {
        assert_eq!(
            reloaded.stages.get(stage_id).unwrap().status,
            StageStatus::Pending,
            "{stage_id} should be reset for rerun after upstream restart"
        );
    }
}

fn test_spec() -> WorkflowSpec {
    WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "test".to_string(),
        task: "Implement a decomposed PRD".to_string(),
        target_repository_root: None,
        max_parallelism: 4,
        max_agents: 16,
        provider_tiers: BTreeMap::from([(ProviderTier::Coder, "test".to_string())]),
        stages: vec![
            test_stage(
                "read-only-review",
                "Read the PRD and inspect current implementation.",
                json!({"task_ids": ["TASK-GEN-001"]}),
                vec![],
            ),
            test_stage(
                "implement-T010-T020",
                "Implement T010 and T020.",
                json!({"task_ids": ["TASK-GEN-010", "TASK-GEN-020"]}),
                vec!["read-only-review".to_string()],
            ),
        ],
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    }
}

fn test_stage(
    id: &str,
    task: &str,
    input: serde_json::Value,
    depends_on: Vec<String>,
) -> StageSpec {
    StageSpec {
        id: id.to_string(),
        kind: StageKind::Agent,
        task: Some(task.to_string()),
        agent: None,
        foreach: None,
        reducer: None,
        tool: None,
        condition: None,
        depends_on,
        provider_tier: Some(ProviderTier::Coder),
        retry: RetryPolicy::default(),
        input,
        model: None,
        provider: None,
        expected_target_files: Vec::new(),
        verify_command: None,
        max_parallelism: None,
        item_kind: None,
        filter: None,
        extra: BTreeMap::new(),
    }
}

fn save_test_branch(v2_store: &WorkflowV2ResultStore, call_id: &str, item_id: &str) {
    let mut result = WorkflowV2Result::accepted(format!("branch {item_id} accepted"));
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!("branch {item_id} has concrete cached evidence"),
    ));
    v2_store
        .save_branch_outcome(
            call_id,
            &WorkflowV2BranchOutcome {
                item_id: item_id.to_string(),
                role: "coder".to_string(),
                status: WorkflowV2Status::Accepted,
                result: Some(result),
                error: None,
                failure_kind: None,
                item_input_hash: Some(format!("test-input-hash-{item_id}")),
                completion_evidence: Vec::new(),
            },
        )
        .unwrap();
}

fn running_stage(id: &str) -> StageState {
    let mut state = StageState::pending(id);
    state.status = StageStatus::Running;
    state
}

fn test_v2_call(
    id: &str,
    method: WorkflowV2HostMethod,
    source: Option<&str>,
) -> WorkflowV2HostCall {
    let options = WorkflowV2HostOptions {
        source: source.map(str::to_string),
        ..Default::default()
    };
    WorkflowV2HostCall {
        id: id.to_string(),
        method,
        write_mode: if method == WorkflowV2HostMethod::Fanout {
            Some(WorkflowV2WriteMode::Coordinated)
        } else {
            None
        },
        options,
    }
}

fn save_test_call_record(
    v2_store: &WorkflowV2ResultStore,
    run_id: &str,
    call: WorkflowV2HostCall,
    depends_on: Vec<String>,
) {
    let mut result = WorkflowV2Result::accepted(format!("{} accepted", call.id));
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        format!("{} has concrete cached evidence", call.id),
    ));
    v2_store
        .save_call_record(&WorkflowV2CallRecord::new(
            run_id,
            call,
            1,
            "test-input-hash".to_string(),
            result,
            depends_on,
        ))
        .unwrap();
}
