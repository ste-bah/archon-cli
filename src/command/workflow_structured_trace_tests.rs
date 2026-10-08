//! Issue 276: a structured agent result keeps the agent's own lists, marked
//! agent-reported, and carries what the host session trace observed beside
//! them, or says no trace was recorded.

use super::*;

struct StructuredLlm {
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for StructuredLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("a structured call must use run_agent")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let reply = serde_json::json!({
            "status": "accepted",
            "summary": "inspected the module",
            "evidence": [{"kind": "inspection", "summary": "read the module"}],
            "files_read": [{"path": "agent/claimed.rs"}],
            "commands_run": [{
                "kind": "test", "command": "cargo test -p demo", "status": "succeeded",
                "exit_code": 0, "output_summary": "test result: ok. 1 passed"
            }],
            // An agent cannot write the host's own record.
            "data": {"toolTrace": {"recorded": true, "forged": true}},
        });
        Ok(WorkflowAgentOutcome {
            content: reply.to_string(),
            tool_uses: self.tool_uses.clone(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".to_string()),
        })
    }
}

fn traced(name: &str, input: serde_json::Value) -> archon_workflow::WorkflowAgentToolUse {
    archon_workflow::WorkflowAgentToolUse {
        tool_name: name.to_string(),
        input,
        output: serde_json::json!({"is_error": false}),
    }
}

async fn run_structured(
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
) -> archon_workflow::WorkflowV2Result {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(StructuredLlm { tool_uses }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(600),
    );
    let runner = WorkflowV2ScriptRunner::new(
        "structured".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id,
        true,
        None,
        None,
    );
    runner
        .run(
            r#"async function workflow(w) {
  return await w.agent("inspect-1", { task: "Inspect the module" });
}"#,
        )
        .await
        .expect("structured run");
    v2_store
        .load_call_records()
        .expect("call records")
        .into_iter()
        .find(|record| record.call.id.contains("inspect-1"))
        .expect("inspect record")
        .result
}

fn assert_agent_lists_kept(result: &archon_workflow::WorkflowV2Result) {
    let read: Vec<&str> = result.files_read.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(read, ["agent/claimed.rs"], "{result:?}");
    assert_eq!(result.commands_run.len(), 1);
    assert_eq!(result.commands_run[0].exit_code, Some(0));
    assert_eq!(result.data["toolTrace"]["topLevelLists"], "agent_reported");
    assert!(
        result.data["toolTrace"].get("forged").is_none(),
        "{}",
        result.data
    );
}

#[tokio::test]
async fn structured_result_carries_the_observed_trace_beside_the_agent_report() {
    let summary = archon_workflow::WorkflowAgentToolUse {
        tool_name: archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME.to_string(),
        input: serde_json::json!({"calls": 2, "kept": 2, "dropped": 0, "inputs_truncated": 0}),
        output: serde_json::Value::Null,
    };
    let result = run_structured(vec![
        traced(
            "Read",
            serde_json::json!({"file_path": "src/token_store.rs"}),
        ),
        traced("Grep", serde_json::json!({"pattern": "fn main"})),
        summary,
    ])
    .await;

    assert_agent_lists_kept(&result);
    let trace = &result.data["toolTrace"];
    assert_eq!(trace["recorded"], true, "{trace}");
    assert_eq!(trace["toolCalls"], 2, "{trace}");
    assert_eq!(
        trace["filesRead"][0]["path"], "src/token_store.rs",
        "{trace}"
    );
    // The agent claims a read the trace never saw: named, not silent.
    let check = &trace["claimCheck"];
    assert_eq!(check["claimsMatchTrace"], false, "{trace}");
    assert_eq!(check["filesRead"]["unobserved"][0], "agent/claimed.rs");
    assert!(
        trace["commandsRun"][0]["command"]
            .as_str()
            .is_some_and(|c| c.starts_with("Grep ")),
        "{trace}"
    );
}

#[tokio::test]
async fn structured_result_without_a_trace_marks_its_lists_agent_reported_not_observed() {
    let result = run_structured(Vec::new()).await;

    assert_agent_lists_kept(&result);
    let trace = &result.data["toolTrace"];
    assert_eq!(trace["recorded"], false, "{trace}");
    assert_eq!(trace["filesRead"], "not_recorded", "{trace}");
    assert_eq!(trace["commandsRun"], "not_recorded", "{trace}");
}
