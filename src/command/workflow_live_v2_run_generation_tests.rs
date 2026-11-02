use super::*;

struct ResumeBeforeReply {
    store: WorkflowStore,
    run_id: String,
    status: WorkflowV2Status,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for ResumeBeforeReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let lifecycle = LifecycleController::new(self.store.clone());
        lifecycle.apply(&self.run_id, LifecycleAction::Pause)?;
        lifecycle.apply(&self.run_id, LifecycleAction::Resume)?;
        let result = WorkflowV2Result {
            status: self.status,
            summary: "reply from the obsolete executor".into(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "read",
            )],
            ..WorkflowV2Result::default()
        };
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result).unwrap(),
            tool_uses: Vec::new(),
            tokens_in: Some(1),
            tokens_out: Some(1),
            stop_reason: None,
        })
    }
}

#[tokio::test]
async fn round3_obsolete_generated_executor_cannot_finalize_resumed_run() {
    for status in [WorkflowV2Status::Accepted, WorkflowV2Status::Failed] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store
            .create_run(super::super::workflow_run_finalizer_tests::spec())
            .unwrap();
        let script = r#"async function workflow(w) {
            await w.agent("inspect-one", {role: "analysis", task: "Inspect the area and report."});
            return {};
        }"#;
        let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
        save_generated_v2_metadata(&store, &run.id, &plan, false).unwrap();
        let llm = Arc::new(ResumeBeforeReply {
            store: store.clone(),
            run_id: run.id.clone(),
            status,
        });
        let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let result = execute_generated_v2_run(
            &store,
            run.clone(),
            plan,
            "test".into(),
            llm,
            ui,
            Vec::new(),
            true,
            false,
            crate::command::workflow_task_root_reclaim::begin_execution(&store, &run.id)
                .map(std::sync::Arc::new)
                .unwrap(),
        )
        .await;
        let current = store.load_state(&run.id).unwrap();
        assert_eq!(current.status, RunStatus::Running, "{status:?}: {result:?}");
        assert_eq!(current.generation, run.generation + 2);
        assert!(
            !store.run_dir(&run.id).join("v2/finalization.json").exists(),
            "{result:?}"
        );
        let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
        assert!(!events.contains("terminal_status"), "{events}");
    }
}

/// The operator acts once, while the first dispatch of the call is in flight.
struct ActionBeforeReply {
    store: WorkflowStore,
    run_id: String,
    action: LifecycleAction,
    dispatches: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for ActionBeforeReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let dispatch = self
            .dispatches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if dispatch == 0 {
            LifecycleController::new(self.store.clone())
                .apply(&self.run_id, self.action.clone())?;
        }
        let result = WorkflowV2Result {
            status: WorkflowV2Status::Accepted,
            summary: format!("inspection completed by dispatch {dispatch}"),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "read the requested area",
            )],
            ..WorkflowV2Result::default()
        };
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: serde_json::to_string(&result)?,
            tool_uses: Vec::new(),
            tokens_in: Some(1),
            tokens_out: Some(1),
            stop_reason: None,
        })
    }
}

async fn assert_running_action_finalizes(action: LifecycleAction) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let mut spec = super::super::workflow_run_finalizer_tests::spec();
    let mut second = spec.stages[0].clone();
    second.id = "call-2".into();
    spec.stages.push(second);
    let mut run = store.create_run(spec).unwrap();
    run.status = RunStatus::Running;
    run.items.insert(
        "item-1".into(),
        archon_workflow::run::ItemState {
            id: "item-1".into(),
            stage_id: "call-1".into(),
            status: archon_workflow::StageStatus::Pending,
            artifact: None,
            error: None,
        },
    );
    store.save_state(&run).unwrap();
    let script = r#"async function workflow(w) {
        await w.agent("inspect-one", {role: "analysis", task: "Inspect the area and report."});
        return {};
    }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_generated_v2_metadata(&store, &run.id, &plan, false).unwrap();
    let llm = Arc::new(ActionBeforeReply {
        store: store.clone(),
        run_id: run.id.clone(),
        action,
        dispatches: Default::default(),
    });
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let result = execute_generated_v2_run(
        &store,
        run.clone(),
        plan,
        "test".into(),
        llm.clone(),
        ui,
        Vec::new(),
        true,
        false,
        crate::command::workflow_task_root_reclaim::begin_execution(&store, &run.id)
            .map(std::sync::Arc::new)
            .unwrap(),
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    // Round 5: the call in flight across the edit is fenced. Its pre-edit
    // result is discarded and the owning executor dispatches it again.
    assert_eq!(
        llm.dispatches.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the pre-edit result must not be published"
    );
    let record = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .load_call_record("inspect-one")
        .unwrap()
        .expect("the re-dispatched call is recorded");
    assert_eq!(record.result.summary, "inspection completed by dispatch 1");
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Completed,
        "{result:?}"
    );
    assert!(store.run_dir(&run.id).join("v2/finalization.json").exists());
    assert!(std::fs::read_to_string(store.events_path(&run.id))
        .unwrap()
        .contains("terminal_status"));
}

#[tokio::test]
async fn round5_restart_stage_fences_the_call_and_keeps_the_executor_owner() {
    assert_running_action_finalizes(LifecycleAction::RestartStage("call-1".into())).await;
}

#[tokio::test]
async fn round5_restart_item_fences_the_call_and_keeps_the_executor_owner() {
    assert_running_action_finalizes(LifecycleAction::RestartItem {
        stage_id: "call-1".into(),
        item_id: "item-1".into(),
    })
    .await;
}

#[tokio::test]
async fn round5_force_accept_stage_fences_the_call_and_keeps_the_executor_owner() {
    assert_running_action_finalizes(LifecycleAction::ForceAcceptStage {
        stage_id: "call-1".into(),
        forced_by: "operator".into(),
        rationale: "reviewed".into(),
        source: "test".into(),
    })
    .await;
}

#[tokio::test]
async fn provider_env_resume_cannot_grant_from_tampered_state_task() {
    if std::env::var_os("ARCHON_R4_GRANT_PROBE").is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([
                "provider_env_resume_cannot_grant_from_tampered_state_task",
                "--nocapture",
            ])
            .env("ARCHON_R4_GRANT_PROBE", "1")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("DECLARED_API_KEY", "synthetic-r4-grant")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let mut spec = super::super::workflow_run_finalizer_tests::spec();
    spec.task = "never expose X_API_KEY".into();
    let mut run = store.create_run(spec).unwrap();
    let script = r#"export default async function workflow(w) { await w.agent("inspect", { task: "Inspect the area." }); }"#;
    WorkflowBundle::create_for_run(&store, &run, script, WorkflowBundleOrigin::GeneratedHarness)
        .unwrap();
    run.spec.task = "required_env_keys: [X_API_KEY]".into();
    let state_path = store.run_dir(&run.id).join("state.json");
    std::fs::write(state_path, serde_json::to_vec(&run).unwrap()).unwrap();
    let resumed = store.load_state(&run.id).unwrap();
    let plan = live_plan_from_generated_bundle(&store, &resumed)
        .await
        .unwrap()
        .unwrap();
    let _ = plan;
    assert!(
        WorkflowBundle::verified_operator_env_grants(&store, &resumed.id)
            .unwrap()
            .is_empty()
    );
    // The same state tampering cannot widen a genuine launch declaration.
    WorkflowBundle::create_for_run_with_operator_grants(
        &store,
        &run,
        script,
        WorkflowBundleOrigin::GeneratedHarness,
        &["DECLARED_API_KEY".into()],
    )
    .unwrap();
    assert_eq!(
        WorkflowBundle::verified_operator_env_grants(&store, &resumed.id).unwrap(),
        if archon_shell::read_boundary::operator_grants_supported() {
            vec!["DECLARED_API_KEY"]
        } else {
            Vec::new()
        }
    );
    let host_record = archon_shell::credential_paths::home()
        .join(".archon/operator-env-grants")
        .join(format!("{}.json", resumed.id));
    assert_eq!(
        host_record.is_file(),
        archon_shell::read_boundary::operator_grants_supported(),
        "issued launch grants must be persisted outside project state"
    );
    let resumed_grants = WorkflowBundle::verified_operator_env_grants(&store, &resumed.id).unwrap();
    let policy =
        archon_tools::provider_env::ProviderEnvPolicy::new(vec!["DECLARED_API_KEY".into()]);
    let resolution =
        archon_tools::provider_env::resolve_operator_provider_env(&policy, &resumed_grants, &[])
            .await;
    use archon_tools::tool::Tool;
    let command = archon_tools::bash::BashTool::default()
        .with_provider_env_resolution(resolution)
        .execute(
            serde_json::json!({"command": if archon_shell::read_boundary::operator_grants_supported() { "test -n \"$DECLARED_API_KEY\" && printf granted" } else { "test -z \"${DECLARED_API_KEY+x}\" && printf withheld" }}),
            &archon_tools::tool::ToolContext {
                working_dir: temp.path().to_path_buf(),
                ..Default::default()
            },
        )
        .await;
    assert!(
        !command.is_error,
        "resumed declared key must reach the command"
    );
    assert!(command.content.contains(
        if archon_shell::read_boundary::operator_grants_supported() {
            "granted"
        } else {
            "withheld"
        }
    ));
    let compiled_path = store
        .run_dir(&run.id)
        .join(archon_workflow::bundle::COMPILED_SPEC_FILE);
    let compiled = std::fs::read_to_string(&compiled_path).unwrap();
    std::fs::write(
        &compiled_path,
        compiled.replace("DECLARED_API_KEY", "X_API_KEY"),
    )
    .unwrap();
    assert_eq!(
        WorkflowBundle::verified_operator_env_grants(&store, &resumed.id).unwrap(),
        if archon_shell::read_boundary::operator_grants_supported() {
            vec!["DECLARED_API_KEY"]
        } else {
            Vec::new()
        }
    );
}

#[tokio::test]
async fn provider_env_resume_r5_refuses_operator_requested_id_state_substitution() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let victim = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    let donor = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    std::fs::write(
        store.state_path(&victim.id),
        serde_json::to_vec(&donor).unwrap(),
    )
    .unwrap();
    let llm = Arc::new(ResumeBeforeReply {
        store: store.clone(),
        run_id: donor.id.clone(),
        status: WorkflowV2Status::Accepted,
    });
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let result = resume_generated_v2_workflow(
        temp.path(),
        &store,
        &victim.id,
        llm,
        ui,
        Vec::new(),
        LiveApprovalMode::CliYes,
        true,
        &archon_core::config::LearningConfig::default(),
    )
    .await;
    assert!(
        result.is_err(),
        "resume must refuse a state ID different from the operator's CLI argument"
    );
    assert!(result.unwrap_err().to_string().contains("run id mismatch"));
}
