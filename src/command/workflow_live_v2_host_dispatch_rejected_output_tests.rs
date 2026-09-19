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

/// A cutoff by the host's own timer leaves a `call_timeout` row beside the
/// `agent_call_failed` one, so it can be told from a provider drop later.
#[test]
fn a_host_timeout_is_recorded_as_call_timeout_in_transport_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("v2").join("transport.jsonl");
    let scope = archon_observability::transport::EvidenceScope::new(path.clone(), "implement-1-0")
        .expect("scope");
    let cutoff = "workflow stage failed: agent transport failed: workflow stage failed: \
                  subagent timed out after 7200s";
    let row = host_call_timeout_record("implement-1-0", cutoff, Some(7200), "host_call_timeout_secs", 7199)
        .expect("a host cutoff is recorded");
    scope.record(row);
    scope.record(serde_json::json!({"kind":"agent_call_failed"}));
    scope.check().expect("durable");

    let rows: Vec<serde_json::Value> = fs::read_to_string(&path)
        .expect("transport evidence")
        .lines()
        .map(|line| serde_json::from_str(line).expect("json row"))
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["kind"], "call_timeout");
    assert_eq!(rows[0]["call_id"], "implement-1-0");
    assert_eq!(rows[0]["limit_secs"], 7200);
    assert_eq!(rows[0]["elapsed_secs"], 7199);
    assert_eq!(rows[0]["source"], "host_call_timeout_secs");
    assert_eq!(rows[1]["kind"], "agent_call_failed");

    // The runner's own turn-boundary wording and the raw author deadline count
    // too; a provider drop does not.
    assert!(host_call_timeout_record("c", "subagent failed: Subagent wall-clock timeout: 7201s elapsed (cap: 7200s) at turn 40/200", Some(7200), "host_call_timeout_secs", 7201).is_some());
    assert!(host_call_timeout_record("c", "author attempt deadline exceeded after 1500s, including transient retries", Some(1500), "host_call_timeout_secs", 1500).is_some());
    assert!(host_call_timeout_record("c", "agent transport failed: response_failed: connection reset", Some(7200), "host_call_timeout_secs", 40).is_none());
    assert!(host_call_timeout_record("c", "agent result failed validation: the agent said it timed out after reading", Some(7200), "host_call_timeout_secs", 40).is_none());
}

/// The predicate that writes the `call_timeout` row is the one that types the
/// error the port returns, so a cut cannot be recorded as the host's and still
/// reach a re-ask loop as a transport failure (Issue-10).
#[test]
fn a_host_cutoff_is_typed_by_the_same_predicate_that_records_it() {
    let pipeline_text = "agent transport failed: subagent timed out after 1800s";
    assert!(is_host_call_timeout(pipeline_text));
    let typed = WorkflowError::HostCallTimeout(pipeline_text.to_string());
    assert!(typed.is_host_call_timeout());
    assert!(is_host_call_timeout(&typed.to_string()), "{typed}");
    assert!(host_call_timeout_record("c", &typed.to_string(), Some(1800), "timeout_retry_budget_secs", 1800).is_some());
    assert!(!archon_workflow::v2::transport_retry::is_transport_failure(&typed.to_string()));
    assert!(!is_host_call_timeout("agent transport failed: subagent failed: HTTP error: response_failed"));
}

/// Issue-54: the tool guard's session-ending text and the write layer's
/// classifier are spelled in two crates that do not depend on each other.
/// This is where both are visible, so this is where they are held together:
/// the guard's cut is neither a host timeout nor a transport failure.
#[test]
fn the_read_wall_thrash_marker_is_one_spelling_across_the_guard_and_the_write_layer() {
    assert_eq!(
        archon_tools::workflow_read_guard::READ_WALL_THRASH_MARKER,
        archon_workflow::error::READ_WALL_THRASH_MARKER
    );
    let text = format!(
        "agent transport failed: subagent failed: {} 16 non-writing calls after the read budget was exhausted; 0 substantive writes",
        archon_tools::workflow_read_guard::READ_WALL_THRASH_MARKER
    );
    assert!(archon_workflow::error::is_read_wall_thrash_text(&text));
    assert!(!is_host_call_timeout(&text), "{text}");
    assert!(!archon_workflow::v2::transport_retry::is_transport_failure(
        &text
    ));
    assert!(
        !archon_workflow::llm_retry::transient_live_agent_error(&text),
        "{text}"
    );
}
