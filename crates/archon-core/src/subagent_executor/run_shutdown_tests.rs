use super::tests::{queued_request, test_executor_with_sink};
use super::*;
use archon_llm::{
    provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature},
    streaming::StreamEvent,
};
use archon_tools::audit_landing::{AuditLanding, LandingHost};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

struct Host(AtomicBool);
impl LandingHost for Host {
    fn accepts_record_arrays(&self) -> bool {
        false
    }
    fn empty_reply_policy(&self) -> archon_tools::audit_landing::EmptyReplyPolicy {
        archon_tools::audit_landing::EmptyReplyPolicy::Resend
    }
    fn receipt_envelope_pointer(&self) -> &'static str {
        "/data/repository_audit"
    }
    fn progress_key_fields(&self) -> &'static [&'static str] {
        &["declared_path"]
    }
    fn allows_unreceipted_reply(&self) -> bool {
        true
    }
    fn normalize_record_reply(&self) -> bool {
        true
    }
    fn tool_name(&self) -> &'static str {
        "land-audit-record"
    }
    fn schema(&self) -> Option<serde_json::Value> {
        None
    }
    fn land(&self, _: Value) -> Result<String, String> {
        self.0.store(true, Ordering::Relaxed);
        Ok("record retained; send receipt".into())
    }
    fn hint(&self) -> Result<String, String> {
        Ok("Send receipt".into())
    }
    fn complete(&self, value: &Value) -> Result<(), String> {
        if self.0.load(Ordering::Relaxed) && *value == json!({"records_landed":1}) {
            Ok(())
        } else {
            Err("no receipt".into())
        }
    }
    fn retained_completion(&self) -> Option<Value> {
        self.0
            .load(Ordering::Relaxed)
            .then(|| json!({"records_landed":1}))
    }
}
struct ShutdownProvider(Arc<tokio::sync::Mutex<crate::subagent::SubagentManager>>);
#[async_trait::async_trait]
impl LlmProvider for ShutdownProvider {
    fn name(&self) -> &str {
        "shutdown-test"
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
    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let manager = self.0.lock().await;
        let active = manager.list_active();
        assert_eq!(active.len(), 1);
        assert!(manager.request_shutdown(&active[0].id));
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(StreamEvent::TextDelta {
            index: 0,
            text: json!({"declared_path":"src/lib.rs"}).to_string(),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: None,
        })
        .await
        .unwrap();
        tx.send(StreamEvent::MessageStop).await.unwrap();
        Ok(rx)
    }
}

#[tokio::test]
async fn subagent_executor_shutdown_with_retained_records_is_control_paused() {
    let sink = archon_observability::InMemoryActivitySink::new();
    let mut executor = test_executor_with_sink(1, Some(Arc::new(sink.clone())));
    let manager = executor.subagent_manager.clone();
    Arc::get_mut(&mut executor).unwrap().client = Arc::new(ShutdownProvider(manager));
    let host = Arc::new(Host(AtomicBool::new(false)));
    let mut request = queued_request();
    request.isolation = Some("shared".into());
    let error = executor
        .run_subagent_to_completion(
            "shutdown-retained".into(),
            request,
            ToolContext {
                audit_landing: Some(Arc::new(AuditLanding::new(host.clone(), None))),
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ExecutorError::ControlPaused(message) if message.contains("shutdown")),
        "{error}"
    );
    assert!(
        host.0.load(Ordering::Relaxed),
        "record remains in host store"
    );
    let statuses: Vec<_> = sink.events().iter().map(|e| e.status).collect();
    assert!(statuses.contains(&archon_observability::AgentActivityStatus::Failed));
    assert!(!statuses.contains(&archon_observability::AgentActivityStatus::Completed));
}
