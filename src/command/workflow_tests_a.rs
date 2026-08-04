use super::*;

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
        decomposed: false,
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
fn run_resume_from_rejects_decomposed_flag() {
    let action = WorkflowAction::Run {
        spec_file: None,
        from_template: None,
        resume_from: Some("prior-run".to_string()),
        decomposed: true,
        live: true,
        yes: true,
        task: vec!["same canary task".to_string()],
    };

    let err = cli_action(&action).expect_err("decomposed resume-from must fail");

    assert!(err.to_string().contains("--decomposed"));
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
fn restart_task_generated_v2_resolves_alias_without_static_stage() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let mut run = store.create_run(test_spec()).unwrap();
    run.stages
        .insert("alpha-write".to_string(), running_stage("alpha-write"));
    run.stages
        .insert("beta-check".to_string(), running_stage("beta-check"));
    run.stages
        .insert("gamma-write".to_string(), running_stage("gamma-write"));
    store.save_state(&run).unwrap();
    WorkflowBundle::create_for_run(
        &store,
        &run,
        "export default async function workflow(w) { await w.agent(\"alpha-write\", { task: \"write\" }); }",
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    let manifest_calls = vec![
        test_v2_call("alpha-write", WorkflowV2HostMethod::Fanout, None),
        test_v2_call(
            "beta-check",
            WorkflowV2HostMethod::Fanout,
            Some("alpha-write.items"),
        ),
        test_v2_call("gamma-write", WorkflowV2HostMethod::Fanout, None),
    ];
    std::fs::create_dir_all(store.run_dir(&run.id).join("v2")).unwrap();
    std::fs::write(
        store.run_dir(&run.id).join("v2/generated-metadata.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "workflow-generated-v2-metadata-v1",
            "task_universe": {
                "schema_version": "workflow-v2-task-universe-v1",
                "source_roots": ["tasks"],
                "tasks": [
                    {
                        "canonical_task_id": "TASK-ALPHA-010",
                        "aliases": ["T010"],
                        "source_path": "tasks/TASK-ALPHA-010.md",
                        "dependency_ids": []
                    },
                    {
                        "canonical_task_id": "TASK-ALPHA-020",
                        "aliases": ["T020"],
                        "source_path": "tasks/TASK-ALPHA-020.md",
                        "dependency_ids": ["TASK-ALPHA-010"]
                    },
                    {
                        "canonical_task_id": "TASK-ALPHA-030",
                        "aliases": ["T030"],
                        "source_path": "tasks/TASK-ALPHA-030.md",
                        "dependency_ids": []
                    }
                ]
            },
            "generated_scaffold": {
                "host_call_manifest": manifest_calls,
            },
        }))
        .unwrap(),
    )
    .unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    save_test_task_call_record(
        &v2_store,
        &run.id,
        test_v2_call("alpha-write", WorkflowV2HostMethod::Fanout, None),
        vec![],
        &["TASK-ALPHA-010"],
        &[],
    );
    save_test_task_call_record(
        &v2_store,
        &run.id,
        test_v2_call(
            "beta-check",
            WorkflowV2HostMethod::Fanout,
            Some("alpha-write.items"),
        ),
        vec!["alpha-write".to_string()],
        &["TASK-ALPHA-020"],
        &["TASK-ALPHA-010"],
    );
    save_test_task_call_record(
        &v2_store,
        &run.id,
        test_v2_call("gamma-write", WorkflowV2HostMethod::Fanout, None),
        vec![],
        &["TASK-ALPHA-030"],
        &[],
    );
    save_test_task_branch(&v2_store, "alpha-write", "alpha-item", "TASK-ALPHA-010");
    save_test_task_branch(&v2_store, "beta-check", "beta-item", "TASK-ALPHA-020");
    save_test_task_branch(&v2_store, "gamma-write", "gamma-item", "TASK-ALPHA-030");

    let output = restart_task_workflow(&store, &run.id, "ALPHA-010").unwrap();

    assert!(output.contains("resolved to TASK-ALPHA-010"));
    assert!(output.contains("TASK-ALPHA-020"));
    assert!(
        v2_store
            .load_branch_outcome("alpha-write", "alpha-item")
            .unwrap()
            .is_none()
    );
    assert!(
        v2_store
            .load_branch_outcome("beta-check", "beta-item")
            .unwrap()
            .is_none()
    );
    assert!(
        v2_store
            .load_branch_outcome("gamma-write", "gamma-item")
            .unwrap()
            .is_some()
    );
    let reloaded = store.load_state(&run.id).unwrap();
    assert_eq!(
        reloaded.stages.get("alpha-write").unwrap().status,
        StageStatus::Pending
    );
    assert_eq!(
        reloaded.stages.get("beta-check").unwrap().status,
        StageStatus::Pending
    );
    assert_eq!(
        reloaded.stages.get("gamma-write").unwrap().status,
        StageStatus::Running
    );
}
