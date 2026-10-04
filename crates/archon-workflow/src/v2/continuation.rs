//! A refused continuation starts an explicit new agent (#241).
//!
//! A continuation runs exactly the completed invocation it continues, or it
//! is refused. The refusal is correct and is not a failure of the call: the
//! workflow drops that session and starts a new agent, in a new session,
//! told the prior attempt's findings. The new agent is never presented as
//! the old one continuing.
use super::agent_adapter::{WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest};

impl WorkflowV2AgentError {
    /// Whether this is a refusal to continue. Only the typed refusal counts:
    /// its words inside any other error (a model value quoted into a
    /// validation error, say) are not one.
    pub fn is_continuation_refusal(&self) -> bool {
        matches!(self, Self::ContinuationRefused(_))
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
            // The new agent's session is the call's session from now on.
            super::repair_session::replace_current(uuid::Uuid::new_v4().to_string());
            client.run_agent_request(request, fresh_prompt).await
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
