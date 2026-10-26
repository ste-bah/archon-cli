use super::{MessageHistory, context_fit::fit_messages_to_budget};
use archon_tools::tool::{Tool, ToolContext};
use archon_tools::workflow_read_guard::WorkflowReadGuard;
use serde_json::{Value, json};
use std::sync::Arc;

fn context(root: &std::path::Path, guard: Arc<WorkflowReadGuard>) -> ToolContext {
    ToolContext {
        working_dir: root.to_path_buf(),
        session_id: "AC-X-001".into(),
        read_result_replay_limit_bytes: archon_tools::READ_REPLAY_CONTEXT_BYTES,
        workflow_read_guard: Some(guard),
        ..ToolContext::default()
    }
}

async fn read(context: &ToolContext, path: &std::path::Path, force: bool) -> String {
    let input = json!({"file_path": path, "limit": 4, "force_refresh": force});
    let guard = context.workflow_read_guard.as_ref().unwrap();
    assert!(guard.before_tool_for(context, "Read", &input).is_none());
    let result = archon_tools::file_read::ReadTool
        .execute(input.clone(), context)
        .await;
    guard.after_tool("Read", &input, !result.is_error, "ok");
    result.content
}

fn read_tool_use(id: &str) -> Value {
    json!({"role":"assistant","content":[{
        "type":"tool_use","id":id,"name":"Read","input":{"file_path":"data.txt"}
    }]})
}

fn read_tool_result(id: &str, content: &str) -> Value {
    json!({"role":"user","content":[{
        "type":"tool_result","tool_use_id":id,"content":content,"is_error":false
    }]})
}

#[tokio::test]
async fn fitting_history_restores_a_range_whose_result_was_dropped() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.txt");
    std::fs::write(&path, "alpha".repeat(800)).unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(40, 20, false, false));
    let context = context(root.path(), guard);
    let content = read(&context, &path, false).await;
    let messages = vec![
        json!({"role":"system","content":"keep"}),
        json!({"role":"user","content":"old evidence ".repeat(4000)}),
        read_tool_use("r1"),
        read_tool_result("r1", &content),
        json!({"role":"user","content":"new request"}),
    ];

    let mut history = MessageHistory::new(messages);
    let (fitted, _) =
        fit_messages_to_budget(history.as_slice(), 10).expect("history should be fitted");
    assert!(!serde_json::to_string(&fitted).unwrap().contains("alpha"));
    history.replace_for_agent(fitted, &context);

    assert_eq!(read(&context, &path, false).await, content);
}

#[tokio::test]
async fn pruning_history_restores_the_same_range_with_full_content() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.txt");
    std::fs::write(&path, "alpha".repeat(800)).unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(40, 20, false, false));
    let context = context(root.path(), guard);
    let first = read(&context, &path, false).await;
    let second = read(&context, &path, true).await;
    let messages = vec![
        read_tool_use("r1"),
        read_tool_result("r1", &first),
        read_tool_use("r2"),
        read_tool_result("r2", &second),
    ];

    let outcome =
        crate::agent::prune::prune_mechanical(&messages, crate::config::PruneConfig::default());
    assert!(outcome.reclaimed_anything());
    let mut history = MessageHistory::new(messages);
    history.replace_for_agent(outcome.messages, &context);

    assert_eq!(read(&context, &path, false).await, first);
}

#[tokio::test]
async fn history_replacement_restores_an_oversized_read_until_budget_exhaustion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.txt");
    std::fs::write(
        &path,
        (0..4)
            .map(|_| "alpha".repeat(20_000))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(2, 20, false, false));
    let context = context(root.path(), guard);
    let content = read(&context, &path, false).await;
    let mut history =
        MessageHistory::new(vec![read_tool_use("r1"), read_tool_result("r1", &content)]);
    history.replace_for_agent(vec![json!({"role":"user","content":"fitted"})], &context);

    let repeated = read(&context, &path, false).await;
    assert_eq!(repeated, content);

    let input = json!({"file_path": path, "limit": 4});
    let refusal = context
        .workflow_read_guard
        .as_ref()
        .unwrap()
        .before_tool_for(&context, "Read", &input)
        .unwrap();
    assert!(refusal.starts_with("read budget exhausted"), "{refusal}");
}

#[tokio::test]
async fn a_parent_thrashing_ends_the_parent_runner() {
    use archon_llm::streaming::StreamEvent;

    let guard = Arc::new(WorkflowReadGuard::new(0, 0, false, false));
    let parent = ToolContext {
        session_id: "AC-X-001".into(),
        workflow_read_guard: Some(guard.clone()),
        ..ToolContext::default()
    };
    for _ in 0..=archon_tools::workflow_read_guard::MAX_NON_WRITING_CALLS_AFTER_WALL {
        let _ = guard.before_tool_for(&parent, "Read", &json!({"file_path":"data.txt"}));
    }
    let terminal = guard
        .terminal_failure_for(&parent)
        .expect("parent call should set its terminal verdict");
    let call = vec![
        StreamEvent::MessageStart {
            id: "m".into(),
            model: "mock".into(),
            usage: Default::default(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: archon_llm::types::ContentBlockType::ToolUse,
            tool_use_id: Some("t".into()),
            tool_name: Some("Read".into()),
        },
        StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: r#"{"file_path":"data.txt"}"#.into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageStop,
    ];
    let provider = Arc::new(crate::subagent::runner::tests::MockProvider::new(vec![
        call,
        crate::subagent::runner::tests::text_response("done"),
    ]));
    let mut runner = crate::subagent::runner::tests::make_runner(provider, 3);
    runner.tool_context = parent;

    let error = runner.run("continue").await.unwrap_err().to_string();
    assert!(error.contains(&terminal), "{error}");
}

#[tokio::test]
async fn child_thrashing_returns_an_error_to_the_child_caller_and_parent_continues() {
    use archon_tools::tool::{PermissionLevel, ToolCapability, ToolResult};

    let guard = Arc::new(WorkflowReadGuard::new(0, 0, false, false));
    let child = ToolContext {
        session_id: "AC-X-001".into(),
        subagent_id: Some("TASK-X-001".into()),
        workflow_read_guard: Some(guard.clone()),
        ..ToolContext::default()
    };
    let parent = ToolContext {
        session_id: "AC-X-001".into(),
        workflow_read_guard: Some(guard.clone()),
        ..ToolContext::default()
    };
    let child_responses = (0..=archon_tools::workflow_read_guard::MAX_NON_WRITING_CALLS_AFTER_WALL)
        .map(|index| {
            crate::subagent::runner::tests::tool_use_response(
                &format!("AC-X-{index:03}"),
                "Read",
                r#"{"file_path":"data.txt"}"#,
            )
        })
        .chain(std::iter::once(
            crate::subagent::runner::tests::text_response("child should stop before this response"),
        ))
        .collect();
    let child_provider = Arc::new(crate::subagent::runner::tests::MockProvider::new(
        child_responses,
    ));
    let mut child_runner = crate::subagent::runner::tests::make_runner(child_provider, 20);
    child_runner.tool_context = child.clone();

    struct ChildFailure(crate::subagent::runner::SubagentRunner);
    #[async_trait::async_trait]
    impl Tool for ChildFailure {
        fn name(&self) -> &str {
            "AC-X-002"
        }
        fn description(&self) -> &str {
            ""
        }
        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }
        async fn execute(&self, _input: Value, _context: &ToolContext) -> ToolResult {
            match self.0.run("continue").await {
                Ok(result) => ToolResult::success(result),
                Err(error) => ToolResult::error(error.to_string()),
            }
        }
        fn permission_level(&self, _input: &Value) -> PermissionLevel {
            PermissionLevel::Safe
        }
        fn capability(&self) -> ToolCapability {
            ToolCapability::HostLocal
        }
    }

    let provider = Arc::new(crate::subagent::runner::tests::MockProvider::new(vec![
        crate::subagent::runner::tests::tool_use_response("t1", "AC-X-002", "{}"),
        crate::subagent::runner::tests::text_response("parent continued"),
    ]));
    let mut runner = crate::subagent::runner::tests::make_runner(provider.clone(), 3);
    runner.tool_context = parent.clone();
    runner.registry = Arc::new({
        let mut tools = crate::dispatch::ToolRegistry::new();
        tools.register(Box::new(ChildFailure(child_runner)));
        tools
    });

    let result = runner.run("continue").await;
    assert_eq!(result.unwrap(), "parent continued");
    let terminal = guard
        .terminal_failure_for(&child)
        .expect("child invocation should retain its terminal verdict");
    let delivered = serde_json::to_string(&provider.requests()[1].messages).unwrap();
    assert!(delivered.contains(&terminal), "{delivered}");
    assert!(delivered.contains("\"is_error\":true"), "{delivered}");
    assert_eq!(guard.terminal_failure_for(&parent), None);
}

#[test]
fn one_thrashing_sibling_does_not_end_another_sibling() {
    let guard = Arc::new(WorkflowReadGuard::new(0, 0, false, false));
    let sibling_a = ToolContext {
        session_id: "AC-X-001".into(),
        subagent_id: Some("TASK-X-001".into()),
        workflow_read_guard: Some(guard.clone()),
        ..ToolContext::default()
    };
    let sibling_b = ToolContext {
        subagent_id: Some("TASK-X-002".into()),
        ..sibling_a.clone()
    };
    for _ in 0..=archon_tools::workflow_read_guard::MAX_NON_WRITING_CALLS_AFTER_WALL {
        let _ = guard.before_tool_for(&sibling_a, "Read", &json!({"file_path":"data.txt"}));
    }

    assert!(guard.terminal_failure_for(&sibling_a).is_some());
    assert_eq!(guard.terminal_failure_for(&sibling_b), None);
    assert!(
        guard
            .before_tool_for(&sibling_b, "Read", &json!({"file_path":"data.txt"}))
            .is_some_and(|reason| !reason
                .starts_with(archon_tools::workflow_read_guard::READ_WALL_THRASH_MARKER))
    );
}
