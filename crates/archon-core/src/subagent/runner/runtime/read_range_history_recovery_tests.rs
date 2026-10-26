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
async fn a_child_thrashing_sets_the_shared_terminal_and_fails_the_parent_runner() {
    use archon_llm::streaming::StreamEvent;

    let guard = Arc::new(WorkflowReadGuard::new(0, 0, false, false));
    let child = ToolContext {
        session_id: "AC-X-001".into(),
        subagent_id: Some("TASK-X-001".into()),
        workflow_read_guard: Some(guard.clone()),
        ..ToolContext::default()
    };
    for _ in 0..=archon_tools::workflow_read_guard::MAX_NON_WRITING_CALLS_AFTER_WALL {
        let _ = guard.before_tool_for(&child, "Read", &json!({"file_path":"data.txt"}));
    }
    let terminal = guard
        .terminal_failure()
        .expect("child call should set terminal");
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
    let mut parent = crate::subagent::runner::tests::make_runner(provider, 3);
    parent.tool_context.workflow_read_guard = Some(guard);

    let error = parent.run("continue").await.unwrap_err().to_string();
    assert!(error.contains(&terminal), "{error}");
}
