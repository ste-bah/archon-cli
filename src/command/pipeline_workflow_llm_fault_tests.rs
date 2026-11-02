use super::*;

#[tokio::test]
async fn host_command_client_construction_keeps_native_transient_io() {
    let temp = tempfile::tempdir().unwrap();
    let config = ArchonConfig::default();
    let env = archon_core::env_vars::load_env_vars_from(&Default::default());
    let factory = SubagentPipelineClientFactory::configured_only(&config, &env);
    let fault = crate::command::workflow_host_fault_test_hooks::inject(
        crate::command::workflow_host_fault_test_hooks::Point::ClientBuild,
        temp.path(),
        Box::new(|| Err(std::io::ErrorKind::Interrupted.into())),
    );
    let error = factory
        .build_client(WorkflowLlmClientRequest {
            cwd: temp.path().to_owned(),
            origin: "judge".into(),
            session_id: "session".into(),
            read_roots: vec![temp.path().to_owned()],
        })
        .await
        .err()
        .expect("the injected client setup failure");
    assert!(fault.fired());
    let error = anyhow::Error::new(error);
    assert_eq!(
        crate::command::workflow_host_fault::classify(&error),
        crate::command::workflow_host_fault::Class::Transient,
        "{error:#}"
    );
}

struct NativeFailure;
#[async_trait::async_trait]
impl archon_pipeline::runner::LlmClient for NativeFailure {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        Err(std::io::Error::from(std::io::ErrorKind::Interrupted).into())
    }
    async fn send_message_with_temperature(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
        _: f64,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        Err(std::io::Error::from(std::io::ErrorKind::Interrupted).into())
    }
    async fn continue_agent(
        &self,
        _: archon_pipeline::runner::AgentExecutionRequest,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        Err(std::io::Error::from(std::io::ErrorKind::Interrupted).into())
    }
}
fn request() -> WorkflowAgentCall {
    use archon_workflow::llm_client_port::{WorkflowAgentSpec, WorkflowAgentToolAccess};
    WorkflowAgentCall {
        session_id: "session".into(),
        task: "operation".into(),
        cwd: None,
        ordinal: 0,
        attempt: 1,
        agent: WorkflowAgentSpec {
            key: "agent".into(),
            display_name: "agent".into(),
            model: "model".into(),
            phase: 1,
            critical: true,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access: WorkflowAgentToolAccess::Full,
        },
        messages: Vec::new(),
        system: Vec::new(),
        tools: Vec::new(),
        allowed_tools: Vec::new(),
        timeout_secs: None,
        disable_auto_background: true,
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        provider_env: None,
    }
}
#[tokio::test]
async fn workflow_decompose_launch_provider_calls_keep_native_faults() {
    let client = PipelineWorkflowLlmClient::arc(Arc::new(NativeFailure));
    let results = [
        client
            .send_message(Vec::new(), Vec::new(), Vec::new(), "model")
            .await,
        client
            .send_message_with_temperature(Vec::new(), Vec::new(), Vec::new(), "model", 0.0)
            .await,
        client.run_agent(request()).await,
        client.continue_agent(request()).await,
    ];
    for (name, result) in ["plain", "sampled", "agent", "continuation"]
        .into_iter()
        .zip(results)
    {
        let error = anyhow::Error::new(result.err().expect("the native provider error"));
        assert_eq!(
            crate::command::workflow_host_fault::classify(&error),
            crate::command::workflow_host_fault::Class::Transient,
            "{name}: {error:#}"
        );
    }
}
