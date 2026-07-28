use super::tool_postprocess_steps::PostprocessFlow;
use super::tool_types::PreflightResult;
use super::*;
use archon_tools::tool::{PermissionLevel, Tool, ToolRunAdmission};
use std::sync::atomic::{AtomicUsize, Ordering};

struct RetryTestTool {
    executions: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for RetryTestTool {
    fn name(&self) -> &str {
        "RetryTest"
    }

    fn description(&self) -> &str {
        "retry test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.executions.fetch_add(1, Ordering::SeqCst);
        ToolResult::success("executed")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
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
    let mut flow = PostprocessFlow::default();
    agent
        .run_post_tool_hooks(&pre, &mut result, &ctx, &mut flow)
        .await;

    assert!(!result.is_error);
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
