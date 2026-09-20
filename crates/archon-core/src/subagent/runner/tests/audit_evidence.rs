use super::*;
use archon_tools::audit_landing::{AuditLanding, LandingHost};
use serde_json::{Value, json};
#[derive(Default)]
struct Host {
    records: Mutex<std::collections::BTreeSet<String>>,
}
impl LandingHost for Host {
    fn land(&self, value: Value) -> Result<String, String> {
        self.records
            .lock()
            .unwrap()
            .insert(value["declared_path"].as_str().unwrap().into());
        self.hint()
    }
    fn hint(&self) -> Result<String, String> {
        Ok(format!(
            "Host-retained audit records: {} of 41",
            self.records.lock().unwrap().len()
        ))
    }
    fn complete(&self, _: &Value) -> Result<(), String> {
        if self.records.lock().unwrap().len() == 41 {
            Ok(())
        } else {
            Err("missing audit paths".into())
        }
    }
}
#[tokio::test]
async fn audit_landing_tool_cannot_run_without_host_capability() {
    let result = archon_tools::audit_landing::LandAuditRecordTool
        .execute(json!({}), &ToolContext::default())
        .await;
    assert!(result.is_error);
}
#[tokio::test]
async fn audit_runner_repairs_incomplete_terminal_reply_without_new_session() {
    let host = Arc::new(Host::default());
    for i in 0..40 {
        host.land(json!({"declared_path":format!("file-{i}")}))
            .unwrap();
    }
    let reply=json!({"status":"accepted","data":{"repository_audit":{"schema_version":1,"snapshot":"sealed","records_landed":41}}}).to_string();
    let provider = Arc::new(MockProvider::new(vec![
        text_response(&reply),
        tool_use_response("land", "land-audit-record", r#"{"declared_path":"last"}"#),
        text_response(&reply),
    ]));
    let mut runner = make_runner(provider.clone(), 4);
    let mut registry = crate::dispatch::create_default_registry(std::env::temp_dir(), None);
    registry.replace(Box::new(archon_tools::audit_landing::LandAuditRecordTool));
    runner.tool_definitions = archon_llm::provider::shared_tools(registry.tool_definitions());
    runner.registry = Arc::new(registry);
    runner.tool_context.audit_landing = Some(Arc::new(AuditLanding::new(host.clone(), Some(30))));
    assert_eq!(runner.run("audit").await.unwrap(), reply);
    assert_eq!(host.records.lock().unwrap().len(), 41);
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        serde_json::to_string(&requests[1].messages)
            .unwrap()
            .contains("missing audit paths")
    );
    assert!(
        serde_json::to_string(&requests[2].messages)
            .unwrap()
            .contains("land-audit-record")
    );
}

struct SlowAudit {
    calls: AtomicU32,
}
#[async_trait::async_trait]
impl LlmProvider for SlowAudit {
    fn name(&self) -> &str {
        "slow-audit"
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
    async fn stream(&self, _: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(16);
        tokio::spawn(async move {
            if call == 0 {
                for _ in 0..12 {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    if tx
                        .send(StreamEvent::MessageDelta {
                            stop_reason: None,
                            usage: None,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            let events = if call == 0 {
                tool_use_response("land", "land-audit-record", r#"{"declared_path":"last"}"#)
            } else {
                text_response("complete")
            };
            for event in events {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}
#[tokio::test(start_paused = true)]
async fn audit_progress_never_aborts_twelve_minute_inflight_request() {
    let host = Arc::new(Host::default());
    let provider = Arc::new(SlowAudit {
        calls: AtomicU32::new(0),
    });
    let mut runner = make_runner(provider.clone(), 4);
    runner.timeout_secs = 21600;
    let mut registry = crate::dispatch::create_default_registry(std::env::temp_dir(), None);
    registry.replace(Box::new(archon_tools::audit_landing::LandAuditRecordTool));
    runner.tool_definitions = archon_llm::provider::shared_tools(registry.tool_definitions());
    runner.registry = Arc::new(registry);
    runner.tool_context.audit_landing =
        Some(Arc::new(AuditLanding::new(host.clone(), Some(21600 / 41))));
    assert_eq!(runner.run("audit 41 paths").await.unwrap(), "complete");
    assert!(host.records.lock().unwrap().contains("last"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}
