use super::*;
use serde_json::json;

fn record(call_id: &str, status: &str, data: serde_json::Value) -> serde_json::Value {
    json!({
        "run_id": "run-x",
        "call": {"id": call_id, "method": "hostCommand", "options": {}},
        "status": status,
        "attempt": 1,
        "input_hash": "h",
        "result": {"data": data},
        "schema_version": "workflow-result-v2"
    })
}

fn store_with(
    records: &[serde_json::Value],
) -> (tempfile::TempDir, archon_workflow::WorkflowV2ResultStore) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("run-x").join("v2");
    let store = archon_workflow::WorkflowV2ResultStore::new(&root);
    for value in records {
        let id = value["call"]["id"].as_str().unwrap();
        let path = store.result_path(id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    }
    (temp, store)
}

fn check(store: &archon_workflow::WorkflowV2ResultStore) -> Result<()> {
    validate_call_directory(&store.root().join("results"), store, true)
}

#[test]
fn a_paused_host_command_record_does_not_block_resume() {
    for reason in ["paused", "host_process_ended", "unstarted"] {
        let (_t, store) = store_with(&[record(
            "c1",
            "needs_review",
            json!({"call_id": "c1", "error": "workflow paused", "interrupted": reason}),
        )]);
        check(&store).unwrap_or_else(|e| panic!("{reason}: {e}"));
    }
}

#[test]
fn an_accepted_record_with_an_interruption_reason_is_still_refused() {
    let (_t, store) = store_with(&[record(
        "c2",
        "accepted",
        json!({"call_id": "c2", "interrupted": "paused"}),
    )]);
    let error = check(&store).unwrap_err().to_string();
    assert!(error.contains("result.data"), "{error}");
}

#[test]
fn a_needs_review_record_without_a_command_result_or_reason_is_still_refused() {
    let (_t, store) = store_with(&[record("c3", "needs_review", json!({"call_id": "c3"}))]);
    let error = check(&store).unwrap_err().to_string();
    assert!(error.contains("result.data"), "{error}");
}
