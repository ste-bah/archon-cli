//! Real runner, executor, pipeline adapter and host refusal propagation.
use super::*;
use archon_core::{subagent::SubagentManager, subagent_executor::AgentSubagentExecutor};
use std::sync::atomic::{AtomicUsize, Ordering};

struct ProseProvider(AtomicUsize);
#[async_trait::async_trait]
impl LlmProvider for ProseProvider {
    fn name(&self) -> &str {
        "fixture"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        panic!("must use the real runner")
    }
    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let events = reply("Finished inspection.");
        let (tx, rx) = tokio::sync::mpsc::channel(events.len());
        for event in events {
            tx.send(event).await.unwrap();
        }
        Ok(rx)
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn host_dispatch_landing_refusal_crosses_real_executor_and_pipeline() {
    let _lock = archon_tools::subagent_executor::executor_test_lock::lock();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let provider = Arc::new(ProseProvider(AtomicUsize::new(0)));
    let (agents, names) =
        crate::command::workflow_live::workflow_live_test_support::fixture_agent_registry(root);
    let executor = AgentSubagentExecutor::new(
        provider.clone(),
        archon_core::dispatch::create_default_registry(root.into(), None),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
        Arc::new(std::sync::RwLock::new(agents)),
        None,
        None,
        root.into(),
        "fixture".into(),
        "fixture".into(),
        vec![],
        Arc::new(tokio::sync::Mutex::new("bypassPermissions".into())),
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "fixture".into(),
            String::new(),
            String::new(),
        )),
    );
    let previous = archon_tools::subagent_executor::get_subagent_executor();
    archon_tools::subagent_executor::install_subagent_executor(Arc::new(executor));
    let pipeline = crate::command::pipeline_workflow_llm::subagent_workflow_client_for_test(
        provider.clone(),
        "fixture",
        root.into(),
        crate::command::pipeline_workflow_llm::TestClientFallback::Forbidden,
    );
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = super::super::super::LiveV2AgentClient::new(
        pipeline,
        sink,
        names,
        "fixture".into(),
        None,
        Some(30),
    )
    .with_fixed_raw_tool_policy(vec!["Read".into()]);
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "skeleton-author-1".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions {
                result_mode: Some(archon_workflow::AgentResultMode::RawOutcome),
                extra: [
                    ("recordLanding".into(), json!("skeleton")),
                    ("openFindings".into(), json!(["repair task"])),
                ]
                .into(),
                ..Default::default()
            },
        },
        input: json!({}),
        depends_on: vec![],
    };
    let store = WorkflowV2ResultStore::new(root.join("v2"));
    let result = super::super::super::run_single_v2_agent_call(
        "Author tasks",
        None,
        &execution,
        &WorkflowV2AgentAdapter::new(),
        &client,
        Some(&store),
        None,
        true,
    )
    .await;
    if let Some(previous) = previous {
        archon_tools::subagent_executor::install_subagent_executor(previous);
    }
    let result = result.unwrap();
    assert_eq!(provider.0.load(Ordering::SeqCst), 3);
    assert_eq!(result.data["invalid_answer"], true);
    assert!(
        result.summary.contains("incomplete landed artifact"),
        "{}",
        result.summary
    );
    assert!(result.data.get("host_dispatch_error").is_none());
    assert!(result.data.get("host_fault_no_verdict").is_none());
}
