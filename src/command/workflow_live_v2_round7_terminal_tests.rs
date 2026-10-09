//! Round 7 (#285): an operator edit while a terminal stop unwinds never turns
//! the stop into a control outcome and never loses a pending call's record.
use super::*;

pub(super) struct NoHostCommands;
#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for NoHostCommands {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}", request.command_id))
    }
    fn record_is_reusable(
        &self,
        _: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(false)
    }
    async fn execute(
        &self,
        _: archon_workflow::HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        Err(WorkflowError::SpecInvalid(
            "no host commands in this test".into(),
        ))
    }
}

async fn edit_during_unwind(edit: fn() -> LifecycleAction, expected: RunStatus) {
    edit_during_unwind_in(edit(), expected.clone(), false).await;
    edit_during_unwind_in(edit(), expected, true).await;
}

async fn edit_during_unwind_in(edit: LifecycleAction, expected: RunStatus, fixed: bool) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let mut run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .expect("run");
    run.status = RunStatus::Running;
    store.save_state(&run).expect("state");
    if fixed {
        super::terminal_test_support::seed_fixed(&store, &run.id, temp.path());
        assert!(
            store
                .run_dir(&run.id)
                .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH)
                .exists(),
            "fixed terminal fixture must contain decomposition/state.json"
        );
    }
    let script = r#"async function workflow(w) {
        await Promise.allSettled([
            w.agent("slow", {role: "analysis", task: "Inspect the area."}),
            w.humanGate("gate", {task: "Require approval"})
        ]);
        await new Promise(() => {});
    }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    if fixed {
        super::terminal_test_support::save_fixed_metadata(&store, &run.id, &plan);
    } else {
        save_generated_v2_metadata(&store, &run.id, &plan, false).expect("metadata");
    }
    let edited = Arc::new(std::sync::Mutex::new(None));
    let observed = edited.clone();
    let edit_store = store.clone();
    let edit_id = run.id.clone();
    let initial_generation = run.generation;
    super::terminal_test_support::on_unwind(
        store.run_dir(&run.id),
        Box::new(move || {
            LifecycleController::new(edit_store.clone())
                .apply(&edit_id, edit)
                .expect("operator edit");
            let state = edit_store.load_state(&edit_id).unwrap();
            assert!(
                state.generation > initial_generation,
                "operator generation changed during unwind"
            );
            *observed.lock().unwrap() = Some((state.generation, state.status));
        }),
    );
    let llm = Arc::new(super::terminal_test_support::PendingReply);
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(60);
    let report = if fixed {
        let executor = Arc::new(NoHostCommands);
        let run = execute_fixed_decomposition_v2_run(
            &store,
            run.clone(),
            plan,
            llm,
            ui,
            Vec::new(),
            executor,
        );
        let result = tokio::time::timeout(HANG_GUARD, run)
            .await
            .expect("fixed run ends");
        assert!(result.is_ok(), "fixed: {result:?}");
        format!("fixed: {result:?}")
    } else {
        let run = execute_generated_v2_run(
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
        );
        let result = tokio::time::timeout(HANG_GUARD, run).await.expect("run ends");
        assert!(result.is_ok(), "{result:?}");
        format!("{result:?}")
    };
    let observed = edited
        .lock()
        .unwrap()
        .clone()
        .expect("operator edit must land during terminal unwind");
    assert!(observed.0 > initial_generation);
    assert_eq!(
        observed.1,
        if expected == RunStatus::Paused {
            RunStatus::Paused
        } else {
            RunStatus::Running
        }
    );
    assert!(!report.contains("cancelled"), "{report}");
    let state = store.load_state(&run.id).expect("durable status");
    assert_eq!(
        state.generation, observed.0,
        "operator edit remains durable"
    );
    assert_eq!(state.status, expected, "{report}");
    if expected == RunStatus::NeedsReview {
        assert!(store.run_dir(&run.id).join("v2/finalization.json").exists());
    }
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let record = v2
        .load_call_record("slow")
        .expect("lookup")
        .expect("pending call recorded");
    assert_eq!(record.status, WorkflowV2Status::NeedsReview);
    assert_eq!(record.result.data["interrupted"], "terminal_host_stop");
    if fixed {
        let projection: archon_workflow::FixedDecompositionStateV1 = serde_json::from_slice(
            &std::fs::read(
                store
                    .run_dir(&run.id)
                    .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(projection.attempts["slow"].interrupted);
        assert_eq!(
            projection.dispositions["slow"],
            archon_workflow::SubjectDisposition::Interrupted
        );
        let log = std::fs::read_to_string(&projection.log_path).unwrap();
        assert!(log.contains("disposition=interrupted"), "{log}");
        if expected == RunStatus::NeedsReview {
            let finalization: archon_workflow::FinalizationRecordV1 = serde_json::from_slice(
                &std::fs::read(store.run_dir(&run.id).join("v2/finalization.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(
                finalization.run_kind,
                archon_workflow::WorkflowRunKind::FixedDecompositionV1
            );
            assert!(finalization.terminal_event_committed);
        }
    }
    assert!(
        std::fs::read_dir(v2.root().join("inflight"))
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
    );
}

#[tokio::test]
async fn round7_restart_during_terminal_unwind_ends_needs_review() {
    edit_during_unwind(
        || LifecycleAction::RestartStage("call-1".into()),
        RunStatus::NeedsReview,
    )
    .await;
}

#[tokio::test]
async fn round7_force_accept_during_terminal_unwind_ends_needs_review() {
    edit_during_unwind(
        || LifecycleAction::ForceAcceptStage {
            stage_id: "call-1".into(),
            forced_by: "operator".into(),
            rationale: "reviewed".into(),
            source: "test".into(),
        },
        RunStatus::NeedsReview,
    )
    .await;
}

#[tokio::test]
async fn round7_pause_during_terminal_unwind_keeps_pause_and_records_call() {
    edit_during_unwind(|| LifecycleAction::Pause, RunStatus::Paused).await;
}

#[tokio::test]
async fn round8_fixed_terminal_fixture_is_realistic() {
    edit_during_unwind_in(
        LifecycleAction::RestartStage("call-1".into()),
        RunStatus::NeedsReview,
        true,
    )
    .await;
}
