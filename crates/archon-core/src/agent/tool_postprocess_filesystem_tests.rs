use super::tool_postprocess_steps::PostprocessFlow;
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn preflight_rejects_symlink_that_escapes_working_tree() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("escape")).unwrap();
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.working_dir = temp.path().to_path_buf();
    let pending = PendingToolCall {
        id: "escaping-symlink".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"printf should-not-run"}"#.into(),
    };

    assert!(
        agent
            .preflight_single_tool(&pending, AgentMode::Normal)
            .await
            .is_none()
    );
    let result =
        std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::ToolCallComplete { name, result, .. } if name == "Bash" => Some(result),
            _ => None,
        });
    assert!(
        result
            .expect("Bash preflight failure")
            .content
            .contains("resolves outside")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn post_tool_symlink_escape_is_persisted_as_a_completion_blocker() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let session_id = "post-tool-symlink-escape-session";
    let store = durable_plan_store(temp.path(), session_id);
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pending = PendingToolCall {
        id: "create-escaping-symlink".into(),
        name: "Bash".into(),
        input_json: format!(
            r#"{{"command":"ln -s '{}' escape"}}"#,
            outside.path().display()
        ),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("Bash preflight must capture a filesystem baseline");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
    assert!(!result.is_error, "fixture command must create the symlink");

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let result =
        std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::ToolCallComplete { name, result, .. } if name == "Bash" => Some(result),
            _ => None,
        });
    assert!(result.expect("Bash completion").is_error);
    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert!(
        plan.execution_evidence
            .observation_failure
            .as_deref()
            .is_some_and(|failure| failure.contains("resolves outside"))
    );
    assert!(plan.reconciliation.iter().any(|entry| {
        entry.status == archon_session::plan::PlanReconciliationStatus::Deviated
            && entry.detail.contains("filesystem observation incomplete")
    }));
}

#[tokio::test]
async fn next_plan_postprocess_retries_pending_observation_failure() {
    use std::sync::atomic::AtomicUsize;

    let temp = tempfile::tempdir().unwrap();
    let session_id = "postprocess-observation-retry-session";
    let store = durable_plan_store(temp.path(), session_id);
    store.fail_next_observation_failure_persistence();
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let fixture_tool = Arc::new(RetryTestTool {
        executions: Arc::new(AtomicUsize::new(0)),
    });
    let failed_observation = PreflightResult {
        tool_name: "RetryTest".into(),
        tool_id: "missing-baseline".into(),
        input: serde_json::json!({}),
        tool_arc: fixture_tool.clone(),
        file_path: None,
        filesystem_effect: archon_tools::tool::WorkingTreeEffect::Arbitrary,
        filesystem_before: None,
        sandbox_prechecked: true,
    };

    agent
        .postprocess_single_tool(
            &failed_observation,
            ToolResult::success("first result"),
            &ctx,
            "test",
            &mut PostprocessFlow::default(),
        )
        .await;
    let first = std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
        AgentEvent::ToolCallComplete { name, result, .. } if name == "RetryTest" => Some(result),
        _ => None,
    });
    assert!(first.expect("first completion").is_error);
    assert!(agent.observation_failure_blocker.is_some());
    store.fail_next_observation_failure_persistence();

    let retry_trigger = PreflightResult {
        tool_name: "RetryTest".into(),
        tool_id: "retry-pending-observation".into(),
        input: serde_json::json!({}),
        tool_arc: fixture_tool,
        file_path: None,
        filesystem_effect: archon_tools::tool::WorkingTreeEffect::None,
        filesystem_before: None,
        sandbox_prechecked: true,
    };
    agent
        .postprocess_single_tool(
            &retry_trigger,
            ToolResult::success("second result"),
            &ctx,
            "test",
            &mut PostprocessFlow::default(),
        )
        .await;
    let second =
        std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::ToolCallComplete { name, result, .. } if name == "RetryTest" => {
                Some(result)
            }
            _ => None,
        });
    assert!(second.expect("second completion").is_error);
    assert!(agent.observation_failure_blocker.is_some());

    agent
        .postprocess_single_tool(
            &retry_trigger,
            ToolResult::success("third result"),
            &ctx,
            "test",
            &mut PostprocessFlow::default(),
        )
        .await;
    let third = std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
        AgentEvent::ToolCallComplete { name, result, .. } if name == "RetryTest" => Some(result),
        _ => None,
    });
    assert!(!third.expect("third completion").is_error);
    assert!(agent.observation_failure_blocker.is_none());
    assert!(
        store
            .load_latest_plan(session_id)
            .unwrap()
            .unwrap()
            .execution_evidence
            .observation_failure
            .is_some()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn background_bash_command_is_observed_after_contained_completion() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "background-observation-session";
    let store = durable_plan_store(temp.path(), session_id);
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pending = PendingToolCall {
        id: "background-write".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"(sleep 0.2; printf delayed > delayed) &"}"#.into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("Bash preflight must capture a filesystem baseline");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        session_id: session_id.into(),
        tool_run_tool_use_id: Some(pending.id.clone()),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
    assert!(!result.is_error, "fixture command must return normally");

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let result =
        std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::ToolCallComplete { name, result, .. } if name == "Bash" => Some(result),
            _ => None,
        });
    let result = result.expect("Bash completion");
    assert!(
        !result.is_error,
        "unexpected Bash error: {}",
        result.content
    );
    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert!(plan.execution_evidence.observation_failure.is_none());
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!temp.path().join("delayed").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn post_tool_non_utf8_path_is_persisted_as_a_completion_blocker() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "post-tool-non-utf8-path-session";
    let store = durable_plan_store(temp.path(), session_id);
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pending = PendingToolCall {
        id: "create-non-utf8-path".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"true"}"#.into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("Bash preflight must capture a filesystem baseline");
    use std::os::unix::ffi::OsStringExt;
    std::fs::write(
        temp.path().join(std::ffi::OsString::from_vec(vec![0xff])),
        b"",
    )
    .expect("fixture must create the non-UTF-8 path");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
    assert!(!result.is_error, "fixture command must succeed");

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let result =
        std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::ToolCallComplete { name, result, .. } if name == "Bash" => Some(result),
            _ => None,
        });
    assert!(result.expect("Bash completion").is_error);
    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert!(
        plan.execution_evidence
            .observation_failure
            .as_deref()
            .is_some_and(|failure| failure.contains("non-UTF-8 path"))
    );
    assert!(plan.reconciliation.iter().any(|entry| {
        entry.status == archon_session::plan::PlanReconciliationStatus::Deviated
            && entry.detail.contains("filesystem observation incomplete")
    }));
}
