use super::*;
use serde_json::json;

#[test]
fn write_read_set_clean_attempt_retains_orientation_and_scopes_retry_to_task() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let path = write_read_set::path(&store, "agents-2-0");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!(
            "{}\n",
            json!({"path":"src/lib.rs","offset":10,"limit":20,"call":3,"hash":"abc"})
        ),
    )
    .unwrap();
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        data: json!({"canonical_task_ids":["TASK-001"]}),
        ..Default::default()
    };
    write_read_set::attach(&store, "agents-2-0", &mut result);
    assert!(
        result.data.get("partial_work").is_none(),
        "no fabricated patch"
    );
    assert_eq!(result.data["workflow_read_set"][0]["offset"], 10);
    assert!(
        result
            .evidence
            .iter()
            .any(|e| e.summary.contains("src/lib.rs"))
    );
    store
        .save_branch_outcome(
            "agents-2",
            &WorkflowV2BranchOutcome {
                item_id: "agents-2-0".into(),
                role: "coder".into(),
                status: result.status,
                result: Some(result),
                error: None,
                failure_kind: None,
                item_input_hash: None,
                completion_evidence: Vec::new(),
            },
        )
        .unwrap();
    let prompt = write_read_set::with_retry_preamble("implement", &store, &["TASK-001".into()]);
    assert!(
        prompt.contains("src/lib.rs")
            && prompt.contains("offset=10")
            && prompt.contains("limit=20")
    );
    assert!(prompt.contains("orientation") && prompt.ends_with("implement"));
    assert_eq!(
        write_read_set::with_retry_preamble("implement", &store, &["TASK-002".into()]),
        "implement"
    );
}

#[test]
fn write_read_set_path_does_not_escape_store() {
    let store = WorkflowV2ResultStore::new("/safe/v2");
    let path = write_read_set::path(&store, "../../outside");
    assert_eq!(
        path.parent().unwrap(),
        std::path::Path::new("/safe/v2/read-sets")
    );
}
