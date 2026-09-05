//! Drive the judge through the production workflow/pipeline/provider adapters.
use super::*;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Receiver;

struct RecordingProvider(Mutex<Vec<serde_json::Value>>);
#[async_trait]
impl LlmProvider for RecordingProvider {
    fn supports_temperature(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "test-provider"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.0.lock().unwrap().push(request.extra);
        let (tx, rx) = tokio::sync::mpsc::channel(3);
        tx.send(StreamEvent::TextDelta { index:0, text: r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"predicate rejects the false state"}]}"#.into() }).await.unwrap();
        tx.send(StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

#[tokio::test]
async fn judge_requests_and_records_zero_temperature_through_live_adapters() {
    let provider = Arc::new(RecordingProvider(Mutex::new(vec![])));
    let client = crate::command::pipeline_workflow_llm::subagent_workflow_client_for_test(
        provider.clone(),
        "judge-test",
        std::env::temp_dir(),
        crate::command::pipeline_workflow_llm::TestClientFallback::Provider,
    );
    let subject: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version":1,"prd":{"path":"p","digest":"d"},
        "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
        "acceptance":[{"id":"AC-X-001","criterion":"output is valid",
          "check":{"kind":"command","command":"./verify-output","cwd":"project_root"},
          "judgment":{"verdict":"refuted","counterexample":"","reason":"","host_call_id":""}}]
    }))
    .unwrap();
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    for _ in 0..2 {
        let judged = judge_contract(client.as_ref(), subject.clone(), &expected)
            .await
            .unwrap();
        let record = serde_json::to_value(&judged.acceptance[0].judgment).unwrap();
        assert_eq!(record["sampling"]["temperature"], 0.0);
        assert_eq!(record["sampling"]["model"], "sonnet");
    }
    let requests = provider.0.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests.iter().all(|r| r["temperature"] == 0.0),
        "{requests:?}"
    );
}
