use super::*;

#[derive(Clone)]
pub(super) struct JudgeClient {
    pub(super) result: Result<String, String>,
}

#[async_trait]
impl WorkflowLlmClient for JudgeClient {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        self.send_message(messages, system, tools, model).await
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert!(tools.is_empty());
        assert_eq!(model, "sonnet");
        assert_eq!(
            messages.len(),
            1,
            "judge needs one provider-valid user message"
        );
        assert_eq!(messages[0]["role"], "user");
        assert!(
            messages[0]["content"]
                .as_str()
                .is_some_and(|text| { text.contains("exactly one decision for every input id") })
        );
        assert!(system.iter().all(|entry| {
            entry["text"]
                .as_str()
                .is_some_and(|text| !text.contains("acceptance contract JSON"))
        }));
        match &self.result {
            Ok(content) => Ok(WorkflowAgentOutcome {
                content: content.clone(),
                stop_reason: Some("end_turn".into()),
                ..WorkflowAgentOutcome::default()
            }),
            Err(message) => Err(WorkflowError::port(std::io::Error::other(message.clone()))),
        }
    }
}
