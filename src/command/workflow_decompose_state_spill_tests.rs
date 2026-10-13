use super::*;
use crate::command::workflow_decompose_state::{FixedCallProjectionKind, project_fixed_call};
use crate::command::workflow_decompose_state_tests::{host_record, seed_state};

fn run_store() -> (tempfile::TempDir, WorkflowStore, &'static str) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-spill";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    (temp, store, run_id)
}

/// A detail whose encoding is exactly `bytes` long.
fn detail_of(bytes: usize) -> Value {
    let empty = serde_json::to_vec(&serde_json::json!({"event": "e", "subject": ""}))
        .unwrap()
        .len();
    serde_json::json!({"event": "e", "subject": "s".repeat(bytes - empty)})
}

#[test]
fn an_event_at_the_cap_is_logged_unchanged_and_one_byte_over_is_spilled() {
    let (_temp, store, run_id) = run_store();
    let at = detail_of(1024);
    assert_eq!(serde_json::to_vec(&at).unwrap().len(), 1024);
    assert_eq!(bounded_event(&store, run_id, 1, &at, 1024).unwrap(), at);
    let spills = store.run_dir(run_id).join(PROGRESS_EVENT_SPILLS_DIR);
    assert!(!spills.exists(), "an event at the cap writes no spill");

    let over = detail_of(1025);
    let view = bounded_event(&store, run_id, 2, &over, 1024).unwrap();
    assert_eq!(view["event"], "e", "short fields stay in the view: {view}");
    assert_eq!(view["detail_truncated"], true);
    assert_eq!(view["detail_bytes"], 1025);
    assert_eq!(
        view["full_detail"],
        "decomposition/progress-event-spills/event-2.json"
    );
    assert!(view.get("subject").is_none(), "{view}");
    assert!(view["subject_digest"].is_string(), "{view}");
    assert!(serde_json::to_vec(&view).unwrap().len() <= 1024);
    let spilled: Value =
        serde_json::from_slice(&std::fs::read(spills.join("event-2.json")).unwrap()).unwrap();
    assert_eq!(spilled, over, "the spill holds the full detail");
}

#[test]
fn a_projection_past_the_cap_spills_instead_of_failing_and_logs_the_full_subject() {
    let (temp, store, run_id) = run_store();
    let log = temp.path().join("tasks/.decompose.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    seed_state(&store, run_id, &log);
    let mut record = host_record(run_id);
    // An acceptance subject is fixed; a skeleton-phase command's subject is
    // its command id. Only a body's subject comes from its outcome.
    let long = format!("TASK-{}", "X".repeat(70 * 1024));
    let request = record.call.options.host_command.as_mut().unwrap();
    request.command_id = "land-task-body".into();
    record.result.data["subjects"] =
        serde_json::json!([{ "taskId": long, "fileName": "TASK-X.md" }]);
    record.result.data["publicationReceipt"]["command_id"] = "land-task-body".into();
    project_fixed_call(&store, run_id, &record, FixedCallProjectionKind::Executed)
        .expect("an oversized progress event must not fail the projection");

    let events = std::fs::read_to_string(store.events_path(run_id)).unwrap();
    let logged: Vec<Value> = events
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|event: &Value| event["detail"]["detail_truncated"] == true)
        .collect();
    assert_eq!(logged.len(), 1, "{events}");
    let view = &logged[0]["detail"];
    assert!(events.lines().all(|line| line.len() < 64 * 1024));
    let spill = store
        .run_dir(run_id)
        .join(view["full_detail"].as_str().unwrap());
    let full: Value = serde_json::from_slice(&std::fs::read(spill).unwrap()).unwrap();
    assert_eq!(
        full["subject"],
        long.as_str(),
        "the spill holds the full subject"
    );
    assert_eq!(full["call_id"], view["call_id"]);
    // The operator log line is written from the full detail: a body
    // subject is digested there, as for any body.
    let operator = std::fs::read_to_string(&log).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(long.as_bytes());
    assert!(
        operator.contains(&format!("subject_digest={digest}")),
        "{operator}"
    );
}
