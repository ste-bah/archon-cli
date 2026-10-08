//! Actual pipeline -> foreground spawn -> executor continuation handoff.
use archon_pipeline::{
    runner::{
        AgentExecutionRequest, AgentInfo, LlmClient, LlmResponse, PipelineType, ToolAccessLevel,
    },
    subagent_adapter::SubagentPipelineClient,
};
use archon_tools::{
    subagent_executor::{
        ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
        install_subagent_executor,
    },
    subagent_request::SubagentRequest,
    tool::ToolContext,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
struct Fallback;
#[async_trait::async_trait]
impl LlmClient for Fallback {
    async fn send_message(
        &self,
        _: Vec<Value>,
        _: Vec<Value>,
        _: Vec<Value>,
        _: &str,
    ) -> anyhow::Result<LlmResponse> {
        panic!("provider forbidden")
    }
}
#[derive(Default)]
struct Executor {
    ids: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl SubagentExecutor for Executor {
    async fn run_to_completion(
        &self,
        _: String,
        _: SubagentRequest,
        _: ToolContext,
        _: CancellationToken,
    ) -> Result<String, ExecutorError> {
        panic!("system path required")
    }
    async fn run_to_completion_with_system(
        &self,
        id: String,
        req: SubagentRequest,
        system: Vec<Value>,
        _: ToolContext,
        _: CancellationToken,
    ) -> Result<String, ExecutorError> {
        let session =
            archon_tools::subagent_session::current_for(&id).expect("session crossed spawn");
        assert!(archon_tools::subagent_session::current_for("unrelated-agent").is_none());
        assert_eq!(system, vec![json!({"type":"text","text":"stable system"})]);
        let mut ids = self.ids.lock().unwrap();
        if session.continuing {
            assert_eq!(id, ids[0]);
            assert_eq!(req.prompt, "validation feedback");
            assert!(!req.prompt.contains("Pipeline Agent Run"));
            let messages = session.history.messages();
            assert_eq!(messages[1]["content"][0]["type"], "tool_result");
            assert_eq!(messages[2]["content"], "invalid answer");
        } else {
            assert!(req.prompt.contains("Pipeline Agent Run"));
            assert!(session.history.messages().is_empty());
            if !ids.is_empty() {
                assert_ne!(id, ids[0]);
            }
            for message in [
                json!({"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{}}]}),
                json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"source bytes"}]}),
                json!({"role":"assistant","content":"invalid answer"}),
            ] {
                session.history.append(&message);
            }
        }
        ids.push(id);
        Ok("done".into())
    }
    async fn on_inner_complete(&self, _: String, _: Result<String, String>) {}
    async fn on_visible_complete(
        &self,
        _: String,
        _: Result<String, String>,
        _: bool,
    ) -> OutcomeSideEffects {
        Default::default()
    }
    fn auto_background_ms(&self) -> u64 {
        1
    }
    fn classify(&self, _: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}
#[tokio::test]
async fn repair_reuses_completed_session_across_spawn_without_second_pipeline_run() {
    let exec = Arc::new(Executor::default());
    install_subagent_executor(exec.clone());
    let client = SubagentPipelineClient::new(Arc::new(Fallback), ToolContext::default());
    let mut req = AgentExecutionRequest {
        session_id: "repair".into(),
        pipeline_type: PipelineType::Workflow,
        task: "inspect".into(),
        cwd: None,
        ordinal: 1,
        attempt: 1,
        agent: AgentInfo {
            key: "author".into(),
            display_name: "author".into(),
            model: "test".into(),
            phase: 1,
            critical: true,
            parallelizable: true,
            quality_threshold: 0.0,
            tool_access_level: ToolAccessLevel::ReadOnly,
        },
        messages: vec![json!({"role":"user","content":"initial"})],
        system: vec![json!({"type":"text","text":"stable system"})],
        tools: vec![],
        allowed_tools: vec!["Read".into()],
        timeout_secs: Some(10),
        disable_auto_background: true,
        read_roots: Vec::new(),
        write_roots: vec![],
        provider_env_resolution: None,
    };
    assert!(client.continue_agent(req.clone()).await.is_err());
    // Issue 276: a response carries the tool calls of its own dispatch only.
    let first = client.run_agent(req.clone()).await.unwrap();
    let names: Vec<_> = first
        .tool_uses
        .iter()
        .map(|t| t.tool_name.as_str())
        .collect();
    let summary = archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME;
    assert_eq!(names, ["Read", summary]);
    assert_eq!(first.tool_uses[0].output, json!({"is_error": false}));
    assert_eq!(first.tool_uses[1].input["calls"], 1);
    req.messages = vec![json!({"role":"user","content":"validation feedback"})];
    // This repair appended no message, so its history was not captured and
    // no trace (not even an empty one) is claimed.
    let repair = client.continue_agent(req.clone()).await.unwrap();
    assert!(repair.tool_uses.is_empty(), "{:?}", repair.tool_uses);
    client.run_agent(req).await.unwrap();
    assert_eq!(exec.ids.lock().unwrap().len(), 3);
}
