//! Exercises the pipeline adapter's policy handoff across the spawned executor.
use std::sync::Arc;
use archon_pipeline::{runner::{AgentExecutionRequest, AgentInfo, LlmClient, LlmResponse, PipelineType, ToolAccessLevel}, subagent_adapter::SubagentPipelineClient};
use archon_tools::{subagent_executor::{SubagentExecutor, ExecutorError, OutcomeSideEffects, SubagentClassification, install_subagent_executor}, subagent_request::SubagentRequest, tool::{ToolContext, Tool}, file_read::ReadTool, glob_tool::GlobTool, grep::GrepTool};
use serde_json::json;
use tokio_util::sync::CancellationToken;
struct Fallback;
#[async_trait::async_trait]
impl LlmClient for Fallback {
    async fn send_message(&self, _:Vec<serde_json::Value>, _:Vec<serde_json::Value>, _:Vec<serde_json::Value>, _: &str) -> anyhow::Result<LlmResponse> { panic!("no provider calls permitted") }
}
struct ToolExecutor;
#[async_trait::async_trait]
impl SubagentExecutor for ToolExecutor {
    async fn run_to_completion(&self, _:String, _:SubagentRequest, ctx:ToolContext, _:CancellationToken) -> Result<String,ExecutorError> {
        assert!(ctx.denied_directory_names.contains(&".archon".into()));
        assert!(ReadTool.execute(json!({"file_path":".archon/stale.rs"}),&ctx).await.is_error);
        for result in [GrepTool.execute(json!({"pattern":"STALE","output_mode":"content"}),&ctx).await,
            GlobTool.execute(json!({"pattern":"**/*.rs"}),&ctx).await] {
            assert!(!result.content.contains("stale.rs"),"{}",result.content);
            assert!(!result.content.contains("STALE"),"{}",result.content);
        }
        assert!(!ReadTool.execute(json!({"file_path":"current.rs"}),&ctx).await.is_error);
        Ok("verified".into())
    }
    async fn on_inner_complete(&self,_:String,_:Result<String,String>) {}
    async fn on_visible_complete(&self,_:String,_:Result<String,String>,_:bool)->OutcomeSideEffects {Default::default()}
    fn auto_background_ms(&self)->u64 {0}
    fn classify(&self,_:&SubagentRequest)->SubagentClassification {SubagentClassification::Foreground}
}
#[tokio::test]
async fn raw_author_policy_reaches_spawned_tools() {
    let root=tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".archon")).unwrap();
    std::fs::write(root.path().join(".archon/stale.rs"),"STALE").unwrap();
    std::fs::write(root.path().join("current.rs"),"current").unwrap();
    install_subagent_executor(Arc::new(ToolExecutor));
    let client=SubagentPipelineClient::new(Arc::new(Fallback),ToolContext{working_dir:root.path().into(),..Default::default()});
    let request=AgentExecutionRequest {
        session_id:"boundary".into(),pipeline_type:PipelineType::Workflow,task:"inspect".into(),cwd:None,ordinal:1,attempt:1,
        agent:AgentInfo{key:"author".into(),display_name:"author".into(),model:"test".into(),phase:1,critical:true,parallelizable:true,quality_threshold:0.0,tool_access_level:ToolAccessLevel::ReadOnly},
        messages:vec![],system:vec![],tools:vec![],allowed_tools:vec!["Read".into(),"Grep".into(),"Glob".into()],timeout_secs:Some(10),disable_auto_background:true,write_roots:vec![],provider_env_resolution:None,
    };
    let result=archon_tools::read_boundary::scope(vec![".archon".into()],client.run_agent(request)).await.unwrap();
    assert_eq!(result.content,"verified");
}
