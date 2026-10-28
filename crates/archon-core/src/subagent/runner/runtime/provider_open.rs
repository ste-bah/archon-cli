//! Keep provider opens and host clock accounting on the same boundary.
use super::*;

pub(super) async fn open(
    runner: &SubagentRunner,
    request: LlmRequest,
) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, archon_llm::provider::LlmError> {
    crate::agent::provider_transport::open(runner.provider.as_ref(), request).await
}

pub(super) fn operational(error: &anyhow::Error) -> bool {
    archon_tools::provider_recovery::enabled()
        && error
            .chain()
            .find_map(|source| source.downcast_ref::<archon_llm::provider::LlmError>())
            .is_some_and(|error| error.is_operational())
}

pub(super) async fn optional_timeout<T>(
    deadline: Option<tokio::time::Instant>,
    work: impl std::future::Future<Output = T>,
) -> Result<T, tokio::time::error::Elapsed> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, work).await,
        None => Ok(work.await),
    }
}

pub(super) fn adjusted_deadline(
    deadline: Option<tokio::time::Instant>,
    credit: &CargoCredit,
    session: &str,
) -> Option<tokio::time::Instant> {
    deadline.map(|deadline| deadline + credit.live(session))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare(
    runner: &SubagentRunner,
    deadline: Option<tokio::time::Instant>,
    window: &ProgressWindow,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    pressure: &mut request_round::PressureState,
    reasoning_encrypted: Option<String>,
    turn: u32,
) -> anyhow::Result<request_round::PreparedRequest> {
    let preparation = crate::agent::provider_transport::with_idle_guard(
        Duration::from_secs(
            runner
                .agent_config()
                .subagent_stream_idle_timeout_secs
                .max(1),
        ),
        Box::pin(prepare_request_round(
            runner,
            messages,
            auto_compact,
            last_known_context_tokens,
            pressure,
            reasoning_encrypted,
            turn,
        )),
    );
    let work = async {
        optional_timeout(deadline, Box::pin(preparation))
            .await
            .map_err(|_| {
                window.stall_error("while preparing an LLM request", turn.saturating_add(1))
            })?
    };
    await_cancellable_inference(work, runner.tool_context.cancel_parent.as_ref()).await
}

pub(super) fn initial_history(runner: &SubagentRunner, initial_prompt: &str) -> MessageHistory {
    // AGT-024: Use initial_messages for resume, or start fresh
    let mut messages = MessageHistory::new(runner.initial_messages.clone().unwrap_or_default());
    let recovering = runner
        .tool_context
        .subagent_id
        .as_deref()
        .and_then(archon_tools::subagent_session::current_for)
        .is_some_and(|session| session.history.is_recovering());
    if !recovering {
        let user_msg = serde_json::json!({
            "role": "user",
            "content": initial_prompt,
        });
        runner.record_transcript(&user_msg);
        messages.push(user_msg);
    }
    messages
}
