//! A refused continuation starts an explicit new agent (#241).
//!
//! A continuation runs exactly the completed invocation it continues, or it
//! is refused. The refusal is correct and is not a failure of the call: the
//! workflow drops that session and starts a new agent, in a new session,
//! told the prior attempt's findings. The new agent is never presented as
//! the old one continuing.
use super::agent_adapter::{WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest};

/// How every refusal to continue begins, wherever it is raised: the executor,
/// the pipeline session cache, or a client that keeps no sessions.
pub const CONTINUATION_REFUSED: &str = "cannot continue agent";

impl WorkflowV2AgentError {
    /// Whether this is a refusal to continue, however the client carried it.
    pub fn is_continuation_refusal(&self) -> bool {
        let marker = CONTINUATION_REFUSED;
        match self {
            Self::ContinuationRefused(_) => true,
            Self::Transport(text) => text.contains(marker),
            _ => false,
        }
    }
}

/// Continue `request`'s completed invocation with `continuation_prompt`, or,
/// when that is refused, start a new agent with `fresh_prompt`.
pub async fn continue_or_start_new<C>(
    client: &C,
    request: &WorkflowV2AgentRequest,
    continuation_prompt: String,
    fresh_prompt: String,
) -> Result<String, WorkflowV2AgentError>
where
    C: WorkflowV2AgentClient + Sync + ?Sized,
{
    match client
        .continue_agent_request(request, continuation_prompt)
        .await
    {
        Err(error) if error.is_continuation_refusal() => {
            super::repair_session::forget_author();
            let session = uuid::Uuid::new_v4().to_string();
            super::repair_session::scope_id(
                session,
                client.run_agent_request(request, fresh_prompt),
            )
            .await
        }
        other => other,
    }
}

/// [`continue_or_start_new`] where the new agent gets the same prompt.
pub async fn continue_or_restart<C>(
    client: &C,
    request: &WorkflowV2AgentRequest,
    prompt: String,
) -> Result<String, WorkflowV2AgentError>
where
    C: WorkflowV2AgentClient + Sync + ?Sized,
{
    continue_or_start_new(client, request, prompt.clone(), prompt).await
}

/// The prompt of a new agent that takes over a call whose agent cannot be
/// continued: the call's own prompt, then the rejected attempt's findings.
pub fn new_agent_prompt(original: &str, repair: &str) -> String {
    if repair.starts_with(original) {
        return repair.to_string();
    }
    format!(
        "{original}\n\n## A previous agent's answer to this call was rejected\n\
         That agent cannot be continued, so you are a new agent on this call. Its \
         rejected output and what was wrong with it follow; produce a correct answer.\n\n{repair}"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_marker_is_the_one_the_executor_and_pipeline_raise() {
        assert_eq!(
            super::CONTINUATION_REFUSED,
            archon_tools::subagent_session::CONTINUATION_REFUSED
        );
    }
}
