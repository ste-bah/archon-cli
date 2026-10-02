//! #236: what the workflow adapter hands the executor for a workspace-bounded
//! call. The isolation value is the shared enum's spelling, so the
//! executor's parser reads it. The call's read roots travel with it. A
//! call without the boundary sends neither.
use archon_pipeline::{
    runner::{
        AgentExecutionRequest, AgentInfo, LlmClient, LlmResponse, PipelineType, ToolAccessLevel,
    },
    subagent_adapter::SubagentPipelineClient,
};
use archon_tools::{
    isolation::Isolation,
    subagent_executor::{
        ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
        install_subagent_executor,
    },
    subagent_request::SubagentRequest,
    tool::ToolContext,
};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct Fallback;
#[async_trait::async_trait]
impl LlmClient for Fallback {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> anyhow::Result<LlmResponse> {
        panic!("no provider calls permitted")
    }
}

#[derive(Default)]
struct Recording(Mutex<Vec<SubagentRequest>>);
#[async_trait::async_trait]
impl SubagentExecutor for Recording {
    async fn run_to_completion(
        &self,
        _: String,
        request: SubagentRequest,
        _: ToolContext,
        _: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.0.lock().unwrap().push(request);
        Ok("recorded".into())
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
        0
    }
    fn classify(&self, _: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}

fn request(tools: &[&str], read_roots: Vec<String>) -> AgentExecutionRequest {
    AgentExecutionRequest {
        session_id: uuid::Uuid::new_v4().to_string(),
        pipeline_type: PipelineType::Workflow,
        task: "inspect".into(),
        cwd: Some(std::env::temp_dir()),
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
        messages: vec![],
        system: vec![],
        tools: vec![],
        allowed_tools: tools.iter().map(|tool| tool.to_string()).collect(),
        timeout_secs: Some(10),
        disable_auto_background: true,
        read_roots,
        write_roots: vec![],
        provider_env_resolution: None,
    }
}

// One test: the executor is process-global.
#[tokio::test]
async fn a_bounded_call_hands_over_the_shared_value_and_its_read_roots() {
    let recording = Arc::new(Recording::default());
    install_subagent_executor(recording.clone());
    let client = SubagentPipelineClient::new(Arc::new(Fallback), ToolContext::default());
    let prd = std::env::temp_dir().join("spec.md").display().to_string();

    client
        .run_agent(request(
            &["__ARCHON_EXACT_TOOLS__", "Read", "Grep", "Glob"],
            vec![prd.clone()],
        ))
        .await
        .unwrap();
    client
        .run_agent(request(&["Read", "Bash"], vec![prd.clone()]))
        .await
        .unwrap();

    let recorded = recording.0.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    let bounded = recorded[0].isolation.as_deref().expect("a bounded call");
    assert_eq!(
        Isolation::parse(bounded, "the handed-over request"),
        Ok(Isolation::WorkspaceBoundary)
    );
    assert_eq!(recorded[0].read_roots, vec![prd]);
    assert_eq!(recorded[1].isolation, None, "a Bash call is not bounded");
    assert!(recorded[1].read_roots.is_empty());
}
