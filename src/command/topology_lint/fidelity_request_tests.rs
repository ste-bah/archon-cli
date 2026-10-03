use super::*;
use archon_llm::provider::LlmRequest;
use sha2::{Digest, Sha256};

struct EnvelopeCritic {
    inner: Arc<Critic>,
    system_text: &'static str,
}

impl EnvelopeCritic {
    fn system(&self, mut system: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        system.push(serde_json::json!({"type":"text", "text":self.system_text}));
        system
    }
}

#[async_trait]
impl WorkflowLlmClient for EnvelopeCritic {
    fn message_request_identity(&self, request: &LlmRequest) -> Option<String> {
        let body = serde_json::json!({
            "messages":request.messages,
            "system":self.system(request.system.clone()),
            "model":request.model,
            "temperature":request.extra["temperature"],
        });
        Some(hex::encode(Sha256::digest(body.to_string())))
    }

    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the audit pins temperature")
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.inner
            .send_message_with_temperature(messages, self.system(system), tools, model, temperature)
            .await
    }
}

#[tokio::test]
async fn retry_reasks_after_effective_system_changes_at_the_same_config_and_head() {
    let temp = corpus();
    let critic = Critic::new();
    for (system_text, expected_calls) in [("clean", 3), ("clean", 3), ("spoof", 6)] {
        let envelope = Arc::new(EnvelopeCritic {
            inner: critic.clone(),
            system_text,
        });
        // Same legacy identity and binary revision throughout; the actual
        // request envelope alone decides whether these verdicts can be reused.
        assert!(envelope.request_identity().is_none());
        let evaluation = evaluate_lint_with_fidelity_resumable(
            temp.path(),
            &LintSource::Tasks(temp.path().join("tasks/PRD-QX-001")),
            archon_core::config::GateMode::Enforce,
            Ok(envelope),
            &[],
            &FreezeResume::none(),
        )
        .await
        .unwrap();
        assert!(
            evaluation.operational_error().is_none(),
            "{}",
            evaluation.report
        );
        assert_eq!(critic.calls(), expected_calls);
    }
}

#[tokio::test]
async fn an_unknown_envelope_is_reasked_without_claiming_saved_progress() {
    let temp = corpus();
    let mut critic = Critic::new();
    Arc::get_mut(&mut critic).unwrap().request = None;
    let resume = FreezeResume::none();
    complete(temp.path(), critic.clone(), &resume).await;
    complete(temp.path(), critic.clone(), &resume).await;
    assert_eq!(critic.calls(), 6);
    assert!(records(temp.path()).is_empty());
    assert_eq!(resume.progress.total(), 0);
}
