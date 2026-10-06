//! Review regressions: takeover after host entry and during terminal awaits.
use super::*;
use crate::command::workflow_live::workflow_script_tools::owner_test_support;

const TOOL: &str = r#"{"id":"probe","options":{"name":"OwnerProbe","input":{}}}"#;

async fn tool_entry_race(pause_only: bool) {
    let (_temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    let (runner, _rx) = runner(&store, &id, Arc::new(PanicLlm), None, None);
    runner
        .v2_store
        .bind_session_executor(store.load_state(&id).unwrap().generation);
    let host = host_of(runner);
    let (tool, runs, _) = owner_test_support::host(false);
    assert!(host.tool_host.set(tool).is_ok());
    let (other_store, other_id) = (store.clone(), id.clone());
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_fixed_persistence::publication_hook::install(
        store.run_dir(&id), Box::new(move || {
            if pause_only {
                LifecycleController::new(other_store).apply(&other_id, LifecycleAction::Pause).unwrap();
            } else { take_over(&other_store, &other_id); }
        }),
    );
    let result = host.execute("runTool".into(), TOOL.into()).await;
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "tool must not dispatch after entry loses control"
    );
    if pause_only {
        assert!(
            matches!(result, Err(WorkflowError::ControlPaused(_))),
            "{result:?}"
        );
    } else {
        assert_stale(&result);
    }
}
#[tokio::test]
async fn review291_tool_takeover_between_entry_and_dispatch() {
    tool_entry_race(false).await;
}
#[tokio::test]
async fn review291_tool_pause_between_entry_and_dispatch() {
    tool_entry_race(true).await;
}
#[tokio::test]
async fn review291_tool_pending_work_stops_on_takeover() {
    let (_temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    let (runner, _rx) = runner(&store, &id, Arc::new(PanicLlm), None, None);
    runner
        .v2_store
        .bind_session_executor(store.load_state(&id).unwrap().generation);
    let host = host_of(runner);
    let (tool, runs, entered) = owner_test_support::host(true);
    assert!(host.tool_host.set(tool).is_ok());
    let host = Arc::new(host);
    tokio::task::LocalSet::new()
        .run_until(async {
            let job = tokio::task::spawn_local(async move {
                host.execute("runTool".into(), TOOL.into()).await
            });
            entered.notified().await;
            take_over(&store, &id);
            let before = snapshot(&store, &id);
            // The caller never polls the tool future again. Only the host's watcher
            // can wake its independent task and abandon the pending execution.
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), job)
                .await
                .expect("pending tool must stop when its executor is replaced")
                .unwrap();
            assert_stale(&result);
            assert_eq!(runs.load(Ordering::SeqCst), 1);
            assert_eq!(snapshot(&store, &id), before);
        })
        .await;
}

async fn lifecycle_await_race(case: u8, final_audit: bool) {
    let (_temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    let (mut runner, _rx) = runner(&store, &id, Arc::new(PanicLlm), None, None);
    runner
        .v2_store
        .bind_session_executor(store.load_state(&id).unwrap().generation);
    runner.initialize_repository_audit().await.unwrap();
    let host = host_of(runner);
    if case == 1 {
        let mut stop = record(&id, "gate");
        stop.status = WorkflowV2Status::NeedsReview;
        host.mark_terminal(&stop, "gate/result.json".into(), "review".into())
            .await;
    }
    // Block either summary or the final audit after the entry ownership fence.
    let audit = host.runner.client.audit.as_ref().unwrap().clone();
    let boundary = if final_audit {
        Some(audit.lock_write_boundary().await)
    } else {
        None
    };
    let held = if final_audit {
        None
    } else {
        Some(host.accumulator.lock().await)
    };
    let outcome = if case == 0 {
        Ok(())
    } else {
        Err(WorkflowError::StageFailed("driver error".into()))
    };
    let finish = host.finish_lifecycle(outcome);
    tokio::pin!(finish);
    tokio::select! { biased; result = &mut finish => panic!("terminal processing did not await: {result:?}"), _ = async {} => {} }
    take_over(&store, &id);
    use archon_workflow::repository_audit::{
        budget::{AuditPolicy, Limit},
        runtime::AuditRuntime,
    };
    let newer = AuditRuntime::initialize(
        store.clone(),
        id.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    newer
        .update(|state| {
            state.attempts = 77;
            state.last_error = Some("new executor's audit".into());
            Ok(())
        })
        .unwrap();
    let before = snapshot(&store, &id);
    drop(held);
    drop(boundary);
    assert_stale(&finish.await);
    assert_eq!(
        snapshot(&store, &id),
        before,
        "all existing audit bytes belong to B"
    );
}
#[tokio::test]
async fn review291_lifecycle_success_takeover_during_summary() {
    lifecycle_await_race(0, false).await;
    lifecycle_await_race(0, true).await;
}
#[tokio::test]
async fn review291_lifecycle_host_stop_takeover_during_summary() {
    lifecycle_await_race(1, false).await;
}
#[tokio::test]
async fn review291_lifecycle_failure_takeover_during_summary() {
    lifecycle_await_race(2, false).await;
}
