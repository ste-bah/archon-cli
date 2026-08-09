use super::*;

#[test]
fn rejected_write_output_is_persisted_under_v2_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let request = write_request("implementation-wave-branch-a");
    let raw = r#"{"status":"accepted","commands_run":[{"kind":"implementation"}]}"#;

    save_rejected_output(
        Some(&store),
        &request,
        "first",
        raw,
        &WorkflowV2AgentError::MalformedOutput("bad schema".to_string()),
    );

    let saved = fs::read_to_string(store.rejected_output_path(&request.call.id))
        .expect("rejected output log");
    let parsed: serde_json::Value = serde_json::from_str(&saved).expect("json log");
    assert_eq!(parsed["branch_id"], request.call.id);
    assert_eq!(parsed["rejections"][0]["attempt"], "first");
    assert_eq!(parsed["rejections"][0]["raw_body"], raw);
}

#[test]
fn patch_error_result_persists_raw_write_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let request = write_request("implementation-wave-branch-b");
    let raw = r#"{"status":"accepted","idempotent_noop":true}"#;
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "patch is empty".to_string(),
        ..WorkflowV2Result::default()
    };
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "invalid_write_branch_output_branch-b".to_string(),
        description: "patch is empty and item did not declare idempotent_noop".to_string(),
        severity: Some("review".to_string()),
    });

    save_rejected_write_result(Some(&store), &request, "first", raw, &result);

    let saved = fs::read_to_string(store.rejected_output_path(&request.call.id))
        .expect("rejected output log");
    let parsed: serde_json::Value = serde_json::from_str(&saved).expect("json log");
    assert_eq!(parsed["rejections"][0]["raw_body"], raw);
}

/// A verification branch's rejected body used to be discarded outright: the
/// persistence path returned early unless the request was write-capable. A live
/// verification stage died to one unrecognised enum value and left nothing to
/// read, so the cause had to be inferred from the error string alone.
#[test]
fn rejected_output_from_a_read_only_branch_is_persisted_too() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let request = read_only_request("verification-wave-verify-task-tdl-010-4-0");
    let raw = r#"{"status":"accepted","evidence":[{"kind":"build","summary":"cargo build ok"}]}"#;

    assert!(
        !request.is_write_capable(),
        "the point of this test is the read-only path"
    );

    save_rejected_output(
        Some(&store),
        &request,
        "first",
        raw,
        &WorkflowV2AgentError::MalformedOutput("unknown variant `build`".to_string()),
    );

    let saved = fs::read_to_string(store.rejected_output_path(&request.call.id))
        .expect("a read-only branch must leave its rejected body on disk");
    let parsed: serde_json::Value = serde_json::from_str(&saved).expect("json log");
    assert_eq!(parsed["branch_id"], request.call.id);
    assert_eq!(parsed["rejections"][0]["raw_body"], raw);
    assert!(
        parsed["rejections"][0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("build")),
        "the rejected body must be stored with the error that rejected it: {parsed:#?}"
    );
}

fn read_only_request(id: &str) -> archon_workflow::WorkflowV2AgentRequest {
    archon_workflow::WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: archon_workflow::WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "verify branch".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({}),
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    }
}

fn write_request(id: &str) -> archon_workflow::WorkflowV2AgentRequest {
    archon_workflow::WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(archon_workflow::WorkflowV2WriteMode::Worktree),
            options: archon_workflow::WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write branch".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({}),
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}
