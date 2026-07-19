use super::*;
use archon_llm::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature,
};
use archon_llm::streaming::StreamEvent;

#[test]
fn pipeline_learning_schema_defaults_to_project_learning_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db = open_pipeline_learning_db(temp.path()).expect("pipeline db");

    archon_learning::schema::ensure_learning_schema(db.as_ref()).expect("governed learning schema");

    assert!(
        temp.path()
            .join(".archon")
            .join("learning-state.db")
            .exists()
    );
    assert!(!temp.path().join(".archon").join("archon-data.db").exists());
}

#[tokio::test]
async fn workflow_cli_subagent_executor_is_installed_with_configured_cap() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut config = ArchonConfig::default();
    config.subagent.max_concurrent = 3;

    install_workflow_cli_subagent_executor(
        &config,
        Arc::new(FakeProvider),
        temp.path(),
        "workflow-cli-test",
        workflow_cli_agent_config(&config, temp.path(), "workflow-cli-test"),
    )
    .await;

    let executor =
        archon_tools::subagent_executor::get_subagent_executor().expect("installed executor");
    assert_eq!(executor.max_concurrency(), Some(3));
}

struct FakeProvider;

#[async_trait::async_trait]
impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "fake-model".to_string(),
            display_name: "Fake Model".to_string(),
            context_window: 8192,
        }]
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Unsupported("fake provider".to_string()))
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Local
    }
}
