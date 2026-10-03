use super::super::workflow_run_finalizer_tests::{events, seed_call, spec, summary};
use super::*;

fn damaged_label_log(tail: &[u8]) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    std::fs::write(store.events_path(&run.id), tail).unwrap();
    emit_observer_event(
        &store,
        &run.id,
        WorkflowEventKind::RunEndAcceptanceObserverStarted,
        "run_end_acceptance_observer_started",
        serde_json::json!({}),
    )
    .unwrap();
    assert!(event_label_exists(&store, &run.id, "run_end_acceptance_observer_started").unwrap());
}

#[test]
fn observer_label_skips_concatenated_events() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    emit_observer_event(
        &store,
        &run.id,
        WorkflowEventKind::RunEndAcceptanceObserverStarted,
        "other",
        serde_json::json!({}),
    )
    .unwrap();
    let raw = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    damaged_label_log(format!("{}{}\n", raw.trim(), raw.trim()).as_bytes());
}

#[test]
fn observer_label_skips_truncated_tail() {
    damaged_label_log(b"{\"seq\":");
}

async fn replay(summary_path: bool, resume: bool) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Failed);
    for attempt in 0..3 {
        if attempt == 1 && resume {
            archon_workflow::v2::run_state_sync::mark_v2_call_running(&store, &run.id, "call-1")
                .unwrap();
        }
        let stages_before = store.load_state(&run.id).unwrap().stages;
        let before = std::fs::read(store.run_dir(&run.id).join("state.json")).unwrap();
        let modified = std::fs::metadata(store.run_dir(&run.id).join("state.json"))
            .unwrap()
            .modified()
            .unwrap();
        if summary_path {
            finalize_summary(
                &store,
                &run.id,
                WorkflowRunKind::FixedOrSavedScript,
                None,
                &summary(WorkflowV2Status::Failed),
                &v2_store,
                None,
                None,
            )
            .await
            .unwrap();
        } else {
            finalize_run_status(
                &store,
                &run.id,
                WorkflowRunKind::FixedOrSavedScript,
                RunStatus::Failed,
                "failed",
                None,
            )
            .unwrap();
        }
        let state = store.load_state(&run.id).unwrap();
        assert_eq!(state.status, RunStatus::Failed);
        if summary_path || attempt == 1 && resume {
            assert_eq!(
                state.stages["call-1"].status,
                archon_workflow::StageStatus::Failed
            );
            assert!(state.stages["call-1"].completed_at.is_some());
        }
        if attempt == 2 || attempt == 1 && !resume {
            assert_eq!(state.stages, stages_before);
            assert_eq!(
                std::fs::read(store.run_dir(&run.id).join("state.json")).unwrap(),
                before
            );
            assert_eq!(
                std::fs::metadata(store.run_dir(&run.id).join("state.json"))
                    .unwrap()
                    .modified()
                    .unwrap(),
                modified
            );
        }
    }
    assert_eq!(
        events(&store, &run.id)
            .iter()
            .filter(|e| e.detail["event"] == "terminal_status")
            .count(),
        1
    );
}

#[tokio::test]
async fn summary_replay_reconciles_running_state() {
    replay(true, true).await;
}
#[tokio::test]
async fn run_status_replay_reconciles_running_state() {
    replay(false, true).await;
}
#[tokio::test]
async fn terminal_replay_does_not_write() {
    replay(true, false).await;
    replay(false, false).await;
}

#[test]
fn observer_label_skips_split_utf8_tail() {
    damaged_label_log(b"{\"detail\":\"\xc3");
}
