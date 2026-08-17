use super::tool_postprocess_steps::PostprocessFlow;
use super::tool_types::PreflightResult;
use super::*;
use archon_tools::bash::BashTool;
use archon_tools::tool::{ToolRunAdmission, WorkingTreeEffect};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn mutating_tool_without_filesystem_baseline_is_rejected_during_postprocess() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "missing-baseline-session";
    let store = durable_plan_store(temp.path(), session_id);
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };

    agent
        .postprocess_single_tool(
            &missing_baseline_bash_pre(),
            ToolResult::success("wrote"),
            &ctx,
            "test",
            &mut PostprocessFlow::default(),
        )
        .await;

    let result =
        std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::ToolCallComplete { name, result, .. } if name == "Bash" => Some(result),
            _ => None,
        });
    assert!(result.expect("Bash completion").is_error);
    assert!(
        store
            .load_latest_plan(session_id)
            .unwrap()
            .unwrap()
            .execution_evidence
            .touched_files
            .is_empty()
    );
}

#[tokio::test]
async fn preflight_blocks_mutator_when_filesystem_baseline_fails() {
    let temp = tempfile::tempdir().unwrap();
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.working_dir = temp.path().join("missing");
    let pending = PendingToolCall {
        id: "missing-root-bash".into(),
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
    let result = result.expect("Bash preflight failure");
    assert!(result.is_error);
    assert!(
        result
            .content
            .contains("filesystem baseline could not be observed")
    );
}

fn missing_baseline_bash_pre() -> PreflightResult {
    PreflightResult {
        tool_name: "Bash".into(),
        tool_id: "missing-baseline-bash".into(),
        input: serde_json::json!({"command": "printf forged > src/planned.rs"}),
        tool_arc: Arc::new(BashTool::default()),
        file_path: Some("src/planned.rs".into()),
        filesystem_effect: WorkingTreeEffect::Arbitrary,
        filesystem_before: None,
        sandbox_prechecked: false,
    }
}

#[tokio::test]
async fn post_tool_retry_readmits_stable_tool_use_id_with_new_attempt() {
    let mut agent = super::tests::test_agent();
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls_for_callback = Arc::clone(&hook_calls);
    let hooks = Arc::new(crate::hooks::HookRegistry::new());
    hooks.register_callback(
        crate::hooks::HookEvent::PostToolUse,
        crate::hooks::HookCallbackEntry {
            name: "retry-once".into(),
            callback: Arc::new(move |_| {
                let retry = hook_calls_for_callback.fetch_add(1, Ordering::SeqCst) == 0;
                crate::hooks::HookResult {
                    retry: Some(retry),
                    ..Default::default()
                }
            }),
            authority: crate::hooks::SourceAuthority::Project,
            timeout_secs: 1,
        },
    );
    agent.set_hook_registry(hooks);

    let executions = Arc::new(AtomicUsize::new(0));
    let pre = PreflightResult {
        tool_name: "RetryTest".into(),
        tool_id: "tool-use-1".into(),
        input: serde_json::json!({}),
        tool_arc: Arc::new(RetryTestTool {
            executions: Arc::clone(&executions),
        }),
        file_path: None,
        filesystem_effect: WorkingTreeEffect::None,
        filesystem_before: None,
        sandbox_prechecked: true,
    };
    let admissions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let admissions_for_callback = Arc::clone(&admissions);
    let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let outcomes_for_callback = Arc::clone(&outcomes);
    let ctx = ToolContext {
        tool_run_parent_action_id: Some("parent-1".into()),
        tool_run_tool_use_id: Some(pre.tool_id.clone()),
        tool_run_attempt: 0,
        tool_run_admission: Some(Arc::new(move |request| {
            admissions_for_callback
                .lock()
                .unwrap()
                .push((request.tool_use_id, request.attempt));
            ToolRunAdmission::Allowed
        })),
        tool_run_outcome: Some(Arc::new(move |outcome| {
            outcomes_for_callback
                .lock()
                .unwrap()
                .push((outcome.tool_use_id, outcome.attempt));
        })),
        ..ToolContext::default()
    };

    let mut result = crate::tool_run_admission::execute_tool_attempt(
        pre.tool_arc.as_ref(),
        pre.input.clone(),
        &ctx,
        pre.sandbox_prechecked,
    )
    .await;
    let mut raw_result = result.clone();
    let mut flow = PostprocessFlow::default();
    agent
        .run_post_tool_hooks(
            &pre,
            &mut raw_result,
            &mut result,
            &ctx,
            "test-model",
            &mut flow,
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(result.content, "executed-2");
    assert_eq!(raw_result.content, "executed-2");
    assert_eq!(executions.load(Ordering::SeqCst), 2);
    assert_eq!(
        *admissions.lock().unwrap(),
        vec![("tool-use-1".into(), 0), ("tool-use-1".into(), 1)]
    );
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![("tool-use-1".into(), 0), ("tool-use-1".into(), 1)]
    );
}

#[tokio::test]
async fn post_tool_output_replacement_does_not_mutate_raw_execution_result() {
    let mut agent = super::tests::test_agent();
    let hooks = Arc::new(crate::hooks::HookRegistry::new());
    hooks.register_callback(
        crate::hooks::HookEvent::PostToolUse,
        crate::hooks::HookCallbackEntry {
            name: "replace-output".into(),
            callback: Arc::new(|_| crate::hooks::HookResult {
                updated_mcp_tool_output: Some(serde_json::json!("forged presentation")),
                ..Default::default()
            }),
            authority: crate::hooks::SourceAuthority::Project,
            timeout_secs: 1,
        },
    );
    agent.set_hook_registry(hooks);
    let pre = PreflightResult {
        tool_name: "RetryTest".into(),
        tool_id: "output-replacement".into(),
        input: serde_json::json!({}),
        tool_arc: Arc::new(RetryTestTool {
            executions: Arc::new(AtomicUsize::new(0)),
        }),
        file_path: None,
        filesystem_effect: WorkingTreeEffect::None,
        filesystem_before: None,
        sandbox_prechecked: true,
    };
    let ctx = ToolContext::default();
    let mut result = ToolResult::success("executed truth");
    let mut raw_result = result.clone();

    agent
        .run_post_tool_hooks(
            &pre,
            &mut raw_result,
            &mut result,
            &ctx,
            "test-model",
            &mut PostprocessFlow::default(),
        )
        .await;

    assert_eq!(result.content, "forged presentation");
    assert_eq!(raw_result.content, "executed truth");
}
