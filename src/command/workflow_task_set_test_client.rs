use super::*;

#[derive(Clone)]
pub(super) struct JudgeClient {
    pub(super) result: Result<String, String>,
}

#[async_trait]
impl WorkflowLlmClient for JudgeClient {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

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
        assert!(
            matches!(messages.len(), 1 | 3),
            "judge uses an initial user turn or a complete user/assistant/user repair turn: {messages:?}"
        );
        assert_eq!(messages[0]["role"], "user");
        assert!(
            messages[0]["content"]
                .as_str()
                .is_some_and(|text| { text.contains("exactly one decision for every input id") })
        );
        if messages.len() == 3 {
            assert_eq!(messages[1]["role"], "assistant");
            assert_eq!(messages[2]["role"], "user");
            assert!(
                messages[2]["content"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("Your reply was rejected:")),
                "repair turn must preserve the rejection reason: {messages:?}"
            );
        }
        assert!(system.iter().all(|entry| {
            entry["text"]
                .as_str()
                .is_some_and(|text| !text.contains("acceptance contract JSON"))
        }));
        // The evidence pass (Issue 275) is not the batch this client
        // scripts: every check's baseline failure is the absent feature.
        let content = messages[0]["content"].as_str().unwrap_or_default();
        if content.contains("[begin untrusted program output]") {
            return Ok(
                crate::command::workflow_task_set::passability_tests_support::all_accepted(content),
            );
        }
        match &self.result {
            Ok(content) => Ok(WorkflowAgentOutcome {
                content: asked_decisions(messages[0]["content"].as_str(), content),
                stop_reason: Some("end_turn".into()),
                ..WorkflowAgentOutcome::default()
            }),
            Err(message) => Err(WorkflowError::port(std::io::Error::other(message.clone()))),
        }
    }
}

/// The scripted decisions for the checks this call asks about (#549: one
/// call per check), when the script answers every one of them; otherwise
/// the script unchanged, so a scripted malformed or incomplete reply stays
/// exactly what the test wrote.
fn asked_decisions(prompt: Option<&str>, scripted: &str) -> String {
    let asked = prompt
        .and_then(|prompt| prompt.rsplit_once("Checks: "))
        .and_then(|(_, checks)| serde_json::from_str::<Vec<serde_json::Value>>(checks).ok())
        .map(|checks| {
            (checks.iter())
                .filter_map(|check| check["id"].as_str().map(str::to_string))
                .collect::<std::collections::BTreeSet<_>>()
        });
    let Ok(mut reply) = serde_json::from_str::<serde_json::Value>(scripted) else {
        return scripted.to_string();
    };
    let (Some(asked), Some(decisions)) = (asked, reply["decisions"].as_array_mut()) else {
        return scripted.to_string();
    };
    let answers =
        |decision: &serde_json::Value| decision["id"].as_str().is_some_and(|id| asked.contains(id));
    if decisions
        .iter()
        .filter(|decision| answers(decision))
        .count()
        != asked.len()
    {
        return scripted.to_string();
    }
    decisions.retain(answers);
    reply.to_string()
}
