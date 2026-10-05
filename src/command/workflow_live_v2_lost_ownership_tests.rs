//! Issue 329: an executor a resume replaced changes nothing and says so.
//! - Its learning record is fenced on ownership like every other write, also
//!   when its script ends normally.
//! - A fixed decomposition is bound to its executor generation at launch, so
//!   no resume between launch and script start leaves it unfenced.
//! - It reports that it lost ownership to a newer executor, not "cancelled".
use super::round7_terminal_tests::NoHostCommands;
use super::terminal_test_support::{PendingReply, save_fixed_metadata, seed_fixed};
use super::*;

const LEARNING: &str = "learning/generated-workflow-events.jsonl";

fn take_over(store: &WorkflowStore, run_id: &str) {
    let lifecycle = LifecycleController::new(store.clone());
    lifecycle.apply(run_id, LifecycleAction::Pause).unwrap();
    lifecycle.apply(run_id, LifecycleAction::Resume).unwrap();
}

/// A run, and executor A's copy of it from launch, after a resume gave the
/// run to executor B.
fn replaced_run() -> (tempfile::TempDir, WorkflowStore, WorkflowRun) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let launched = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    take_over(&store, &launched.id);
    let owner = store.load_state(&launched.id).unwrap();
    assert!(owner.executor_generation > Some(launched.generation));
    (temp, store, launched)
}

fn state_and_events(store: &WorkflowStore, run_id: &str) -> (String, String) {
    (
        std::fs::read_to_string(store.state_path(run_id)).unwrap(),
        std::fs::read_to_string(store.events_path(run_id)).unwrap_or_default(),
    )
}

async fn run_replaced_generated(
    script: &str,
) -> (tempfile::TempDir, WorkflowStore, String, String) {
    let (temp, store, launched) = replaced_run();
    let plan = WorkflowScriptPlan::from_template(launched.spec.clone(), script, Vec::new());
    save_generated_v2_metadata(&store, &launched.id, &plan, false).unwrap();
    let before = state_and_events(&store, &launched.id);
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let report = execute_generated_v2_run(
        &store,
        launched.clone(),
        plan,
        "test".into(),
        Arc::new(PendingReply),
        ui,
        Vec::new(),
        true,
        false,
        crate::command::workflow_task_root_reclaim::begin_execution(&store, &launched.id)
            .map(Arc::new)
            .unwrap(),
    )
    .await
    .expect("a replaced executor ends without an error");
    assert_eq!(
        state_and_events(&store, &launched.id),
        before,
        "{report}: the run is as executor B left it"
    );
    (temp, store, launched.id, report)
}

fn assert_lost_ownership(report: &str, label: &str, run_id: &str) {
    assert!(
        report.starts_with(&format!(
            "{label} {run_id}: this session lost ownership of the run to a newer executor and changed nothing after that"
        )),
        "{report}"
    );
    assert!(!report.contains("cancelled:"), "{report}");
}

/// B takes the run over after A's script ended normally and A's finalizer
/// committed, before A appends its learning record.
#[tokio::test]
async fn a_replaced_executor_whose_script_ends_normally_appends_no_learning_record() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    let plan = WorkflowScriptPlan::from_template(
        run.spec.clone(),
        "async function workflow(w) { return {}; }",
        Vec::new(),
    );
    save_generated_v2_metadata(&store, &run.id, &plan, false).unwrap();
    let taken: Arc<std::sync::Mutex<Option<(String, String)>>> = Arc::default();
    let (hook_store, hook_id, hook_taken) = (store.clone(), run.id.clone(), taken.clone());
    super::terminal_test_support::on_unwind(
        store.run_dir(&run.id),
        Box::new(move || {
            LifecycleController::new(hook_store.clone())
                .apply(&hook_id, LifecycleAction::Resume)
                .unwrap();
            *hook_taken.lock().unwrap() = Some(state_and_events(&hook_store, &hook_id));
        }),
    );
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();

    let report = execute_generated_v2_run(
        &store,
        run.clone(),
        plan,
        "test".into(),
        Arc::new(PendingReply),
        ui,
        Vec::new(),
        true,
        false,
        crate::command::workflow_task_root_reclaim::begin_execution(&store, &run.id)
            .map(Arc::new)
            .unwrap(),
    )
    .await
    .expect("a replaced executor ends without an error");

    let taken = taken.lock().unwrap().clone().expect("the takeover ran");
    assert!(
        !store.run_dir(&run.id).join(LEARNING).exists(),
        "{report}: no learning record"
    );
    assert_eq!(state_and_events(&store, &run.id), taken, "{report}");
    assert_lost_ownership(&report, "Workflow", &run.id);
}

#[tokio::test]
async fn a_replaced_generated_executor_reports_lost_ownership_not_a_cancel() {
    let (_temp, store, run_id, report) = run_replaced_generated(
        r#"async function workflow(w) { await w.checkpoint("late", {}); return {}; }"#,
    )
    .await;

    assert_lost_ownership(&report, "Workflow", &run_id);
    assert!(report.contains("no longer owns"), "{report}");
    assert!(!store.run_dir(&run_id).join(LEARNING).exists());
}

/// The window the late binding left: B takes over after A's launch but
/// before A's script starts. A's session is still A's.
#[tokio::test]
async fn a_fixed_executor_is_fenced_from_its_launch_not_from_its_script_start() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let mut run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    let script =
        r#"async function workflow(w) { await w.checkpoint("after-takeover", {}); return {}; }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    let v2 = super::super::workflow_live_v2_fixed_run::persist_fixed_start(&store, &mut run)
        .expect("A launches");
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let runner = super::super::workflow_live_v2_fixed_run::fixed_runner(
        &run,
        &plan,
        Arc::new(PendingReply),
        ui,
        Vec::new(),
        Arc::new(NoHostCommands),
        &store,
        &v2,
    );
    take_over(&store, &run.id);

    let outcome = runner.run(&plan.harness_source).await;

    assert!(
        matches!(&outcome, Err(WorkflowError::ControlCancelled(message))
            if message.contains("no longer owns") && message.contains("stale session")),
        "A is refused as a stale session: {outcome:?}"
    );
    assert!(
        v2.load_call_record("after-takeover").unwrap().is_none(),
        "A wrote no record"
    );
}

#[tokio::test]
async fn a_fixed_launch_after_a_newer_executor_took_over_changes_nothing() {
    let (temp, store, launched) = replaced_run();
    seed_fixed(&store, &launched.id, temp.path());
    let plan = WorkflowScriptPlan::from_template(
        launched.spec.clone(),
        "async function workflow(w) { return {}; }",
        Vec::new(),
    );
    save_fixed_metadata(&store, &launched.id, &plan);
    let before = state_and_events(&store, &launched.id);
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();

    let report = execute_fixed_decomposition_v2_run(
        &store,
        launched.clone(),
        plan,
        Arc::new(PendingReply),
        ui,
        Vec::new(),
        Arc::new(NoHostCommands),
    )
    .await
    .expect("a replaced launch ends without an error");

    assert_lost_ownership(&report, "Fixed decomposition", &launched.id);
    assert_eq!(state_and_events(&store, &launched.id), before);
}

#[tokio::test]
async fn a_fixed_executor_replaced_mid_run_reports_lost_ownership_not_a_cancel() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    seed_fixed(&store, &run.id, temp.path());
    let script = r#"async function workflow(w) { await w.checkpoint("first", {}); return {}; }"#;
    let plan = WorkflowScriptPlan::from_template(run.spec.clone(), script, Vec::new());
    save_fixed_metadata(&store, &run.id, &plan);
    let taken: Arc<std::sync::Mutex<Option<(String, String)>>> = Arc::default();
    let (hook_store, hook_id, hook_taken) = (store.clone(), run.id.clone(), taken.clone());
    // B takes the run over while A publishes its first call.
    super::super::workflow_live_v2_fixed_persistence::publication_hook::install(
        store.run_dir(&run.id),
        Box::new(move || {
            take_over(&hook_store, &hook_id);
            *hook_taken.lock().unwrap() = Some(state_and_events(&hook_store, &hook_id));
        }),
    );
    let (ui, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();

    let report = execute_fixed_decomposition_v2_run(
        &store,
        run.clone(),
        plan,
        Arc::new(PendingReply),
        ui,
        Vec::new(),
        Arc::new(NoHostCommands),
    )
    .await
    .unwrap();

    assert_lost_ownership(&report, "Fixed decomposition", &run.id);
    let taken = taken.lock().unwrap().clone().expect("the takeover ran");
    assert_eq!(state_and_events(&store, &run.id), taken, "{report}");
}
