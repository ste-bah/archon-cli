//! Issue 276: a raw-outcome agent result records what the session's tool
//! trace shows it read and ran, or says plainly that no trace was recorded.

use super::*;

struct TracedRawLlm {
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for TracedRawLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed raw outcome must use run_agent")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        Ok(WorkflowAgentOutcome {
            content: "opaque candidate bytes".to_string(),
            tool_uses: self.tool_uses.clone(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".to_string()),
        })
    }
}

fn tool_use(
    name: &str,
    input: serde_json::Value,
    is_error: bool,
) -> archon_workflow::WorkflowAgentToolUse {
    archon_workflow::WorkflowAgentToolUse {
        tool_name: name.to_string(),
        input,
        output: serde_json::json!({ "is_error": is_error }),
    }
}

async fn run_raw_author(
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
) -> archon_workflow::WorkflowV2Result {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(TracedRawLlm { tool_uses }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(600),
    )
    .with_fixed_raw_tool_policy(vec!["Read".into(), "Grep".into(), "Bash".into()]);
    let runner = WorkflowV2ScriptRunner::new(
        "raw author".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id,
        true,
        None,
        None,
    )
    .with_raw_outcomes(true);
    runner
        .run(
            r#"async function workflow(w) {
  return await w.agent("acceptance-author-1", { task: "Author", tier: "planner", resultMode: "rawOutcome" });
}"#,
        )
        .await
        .expect("raw outcome run");
    let records = v2_store.load_call_records().expect("call records");
    records
        .into_iter()
        .find(|record| record.call.id.contains("acceptance-author-1"))
        .expect("author record")
        .result
}

#[tokio::test]
async fn raw_outcome_result_records_files_read_and_commands_run_from_the_tool_trace() {
    let result = run_raw_author(vec![
        tool_use("Read", serde_json::json!({"file_path": "src/a.rs"}), false),
        tool_use("Read", serde_json::json!({"file_path": "src/b.rs"}), false),
        tool_use("Read", serde_json::json!({"file_path": "src/a.rs"}), false),
        tool_use(
            "Grep",
            serde_json::json!({"pattern": "fn main", "path": "src"}),
            false,
        ),
        tool_use(
            "Bash",
            serde_json::json!({"command": "cargo metadata"}),
            true,
        ),
    ])
    .await;

    let read: Vec<&str> = result.files_read.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(read, ["src/a.rs", "src/b.rs"], "{result:?}");
    let commands: Vec<&str> = result
        .commands_run
        .iter()
        .map(|c| c.command.as_str())
        .collect();
    assert!(
        commands
            .iter()
            .any(|c| c.contains("Grep") && c.contains("fn main")),
        "{commands:?}"
    );
    let bash = result
        .commands_run
        .iter()
        .find(|c| c.command.contains("cargo metadata"))
        .expect("the Bash call is recorded");
    assert_eq!(
        bash.status,
        archon_workflow::WorkflowV2CommandStatus::Failed
    );
    assert_eq!(
        result.data["toolTrace"]["recorded"], true,
        "{}",
        result.data
    );
    assert_eq!(result.data["toolTrace"]["toolCalls"], 5, "{}", result.data);
}

#[tokio::test]
async fn raw_outcome_without_a_tool_trace_says_not_recorded_instead_of_none() {
    let result = run_raw_author(Vec::new()).await;

    assert!(result.files_read.is_empty());
    assert!(result.commands_run.is_empty());
    assert_eq!(
        result.data["toolTrace"]["recorded"], false,
        "{}",
        result.data
    );
    assert_eq!(
        result.data["toolTrace"]["filesRead"], "not_recorded",
        "{}",
        result.data
    );
    assert_eq!(
        result.data["toolTrace"]["commandsRun"], "not_recorded",
        "{}",
        result.data
    );
}
