//! REM-14: the host's completion plan, from its own records.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::script::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2WriteMode};

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, files: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: files.iter().map(|f| format!("`{f}` -- the lane")).collect(),
        focused_tests: vec![format!("cargo test {id}")],
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-A", &["a.rs"]),
            task("TASK-B", &["b.rs"]),
            task("TASK-C", &[]),
        ],
    }
}

fn write_record(id: &str, task: &str) -> WorkflowV2CallRecord {
    let options = WorkflowV2HostOptions {
        target_files_from_item: true,
        ..WorkflowV2HostOptions::default()
    };
    let call = WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options,
    };
    let result = WorkflowV2Result {
        data: json!({ "outcomes": [{ "item_id": id, "canonical_task_ids": [task], "status": "accepted" }] }),
        ..WorkflowV2Result::accepted("written")
    };
    WorkflowV2CallRecord::new("run", call, 1, "input".into(), result, Vec::new())
}

fn checkpoint(asks: bool) -> WorkflowV2CallRecord {
    let mut options = WorkflowV2HostOptions::default();
    if asks {
        options
            .extra
            .insert(TASK_COMPLETION_MARKER.into(), Value::Bool(true));
    }
    let call = WorkflowV2HostCall {
        id: "task-completion".into(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    };
    WorkflowV2CallRecord::new(
        "run",
        call,
        1,
        "input".into(),
        WorkflowV2Result::accepted("checkpoint"),
        Vec::new(),
    )
}

fn planned(store: &WorkflowV2ResultStore) -> Vec<String> {
    completion_plan(store, Some(&universe()))
        .iter()
        .map(|entry| entry["task_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn every_task_no_write_of_this_session_named_is_planned_with_what_it_declares() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("v2"));
    store
        .save_call_record(&write_record("agents-1", "task-a"))
        .unwrap();
    let plan = completion_plan(&store, Some(&universe()));
    let ids: Vec<&str> = plan
        .iter()
        .map(|e| e["task_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["TASK-B", "TASK-C"],
        "a write naming TASK-A (any case) covers it"
    );
    let b = &plan[0];
    assert_eq!(b["source"], json!("host"));
    assert_eq!(b["unit"], json!(completion_unit("TASK-B")));
    assert_eq!(b["target_files"], json!(["b.rs"]));
    assert_eq!(b["focused_tests"], json!(["cargo test TASK-B"]));
    assert_eq!(b["task_file"], json!("tasks/TASK-B.md"));
    assert_eq!(b["mode"], json!(COMPLETION_MODE_WRITE));
    // A task that declares no file it may write gets a unit too: one that
    // can only verify it as a no-op, never one silently left out.
    assert_eq!(plan[1]["mode"], json!(COMPLETION_MODE_NOOP_VERIFY));
    assert_eq!(plan[1]["target_files"], json!([]));
}

#[test]
fn a_completion_units_own_writes_never_take_its_task_out_of_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("v2"));
    let unit = completion_unit("TASK-B");
    store
        .save_call_record(&write_record(&format!("{unit}-impl-1"), "TASK-B"))
        .unwrap();
    assert!(is_completion_call(&format!("{unit}-impl-1")));
    assert!(is_completion_call(&format!(
        "verification-wave-{unit}-verify-1"
    )));
    assert!(!is_completion_call("agents-1"));
    // The prefix is reserved: no slug an authored label becomes starts with
    // it, even one written to look like a completion unit's.
    assert!(unit.starts_with("__host-complete-"), "{unit}");
    assert!(!is_completion_call("complete-task-b-impl-1"));
    assert!(!is_completion_call("host-complete-task-b-impl-1"));
    // So a resumed session is planned the same units, and replays them.
    assert_eq!(planned(&store), ["TASK-A", "TASK-B", "TASK-C"]);
}

#[test]
fn a_write_only_an_earlier_session_recorded_does_not_cover_its_task() {
    let dir = tempfile::tempdir().unwrap();
    let earlier = WorkflowV2ResultStore::new(dir.path().join("v2"));
    earlier
        .save_call_record(&write_record("agents-1", "TASK-A"))
        .unwrap();
    let now = WorkflowV2ResultStore::new(dir.path().join("v2"));
    assert_eq!(planned(&now), ["TASK-A", "TASK-B", "TASK-C"]);
    now.note_session_call("agents-1");
    assert_eq!(planned(&now), ["TASK-B", "TASK-C"]);
}

#[test]
fn only_the_asking_checkpoint_carries_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("v2"));
    let asking = checkpoint(true);
    let viewed = with_task_completion(&asking, &asking.result, &store, Some(&universe()))
        .expect("the asking checkpoint is viewed");
    assert_eq!(
        viewed.data[TASK_COMPLETION_KEY].as_array().unwrap().len(),
        3
    );
    let other = checkpoint(false);
    assert!(with_task_completion(&other, &other.result, &store, Some(&universe())).is_none());
    // A plan key some other record carries is stripped: the key is the host's.
    let mut forged = other.result.clone();
    forged.data = json!({ TASK_COMPLETION_KEY: [{ "source": "host", "task_id": "TASK-X" }] });
    let viewed = with_task_completion(&other, &forged, &store, Some(&universe())).unwrap();
    assert!(viewed.data.get(TASK_COMPLETION_KEY).is_none());
}
