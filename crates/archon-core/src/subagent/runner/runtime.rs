use super::*;

mod cargo_credit;
mod context_fit;
#[cfg(test)]
mod context_fit_tests;
mod message_history;
mod progress_stop;
mod progress_window;
#[cfg(test)]
mod progress_window_tests;
mod request_round;
mod request_round_pressure;
mod stream_idle_window;
mod stream_resend_budget;
mod stream_round;
mod tool_helpers;
mod tool_round;

use cargo_credit::CargoCredit;
use message_history::MessageHistory;
use progress_window::ProgressWindow;
use request_round::{PressureState, prepare_request_round};
use stream_round::collect_stream_round;
use tool_round::replay_tool_round;

impl SubagentRunner {
    /// Run the subagent loop with the given initial prompt.
    /// Returns the accumulated text output from the final turn.
    pub async fn run(&self, initial_prompt: &str) -> anyhow::Result<String> {
        // The session has started: from here, and not while it was queued for
        // a subagent slot, silence counts against the host's inactivity bound.
        archon_tools::subagent_activity::note();
        // AGT-024: Use initial_messages for resume, or start fresh
        let mut messages = MessageHistory::new(self.initial_messages.clone().unwrap_or_default());
        let user_msg = serde_json::json!({
            "role": "user",
            "content": initial_prompt,
        });
        self.record_transcript(&user_msg);
        messages.push(user_msg);

        // Issue 288: the session's limit is a no-progress window, never a
        // total. It renews only on novel activity (see `progress_window`).
        let timeout_secs = match archon_tools::host_timeout::current() {
            Some(archon_tools::host_timeout::HostTimeout::Unlimited) => None,
            Some(archon_tools::host_timeout::HostTimeout::Finite(seconds)) => Some(seconds),
            None => Some(self.timeout_secs),
        };
        let mut window = ProgressWindow::open(timeout_secs)?;
        let mut cargo_credit = CargoCredit::new(timeout_secs);
        let mut progress_stop = progress_stop::ProgressStop::default();
        let progress_agent = self.tool_context.subagent_id.clone().unwrap_or_default();
        let mut auto_compact = crate::agent::AutoCompactState::default();
        let mut cumulative_billable_tokens = 0_u64;
        let mut last_known_context_tokens = 0_u64;
        let mut reasoning_encrypted: Option<String> = None;
        let mut recovery_ladder = crate::agent::autocompact::RecoveryLadder::default();
        let mut emergency_projection_pending = false;
        let mut reactive_rate_limit_retried = false;
        let mut pressure = PressureState::default();
        let mut incomplete_audit_replies = 0u8;

        for turn in 0..self.max_turns {
            // Issue-213 C5: the turn an interrupted call's record names.
            archon_tools::session_progress::note_turn(&progress_agent, turn.saturating_add(1));
            // The no-progress window ran out between turns. Its text names
            // the window and the last novel activity, so a reader can tell a
            // looping session from a slow one.
            if window.expired() {
                return Err(window.stall_error("between turns", turn.saturating_add(1)));
            }

            // Check for graceful shutdown request
            if self
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Ok("[Agent shutdown requested]".to_string());
            }

            if let Some(landing) = &self.tool_context.audit_landing
                && let Some(text) = landing.progress_message().map_err(anyhow::Error::msg)?
            {
                let message = serde_json::json!({"role":"user","content":text});
                self.record_transcript(&message);
                messages.push(message);
            }
            let request_deadline = adjusted_deadline(
                window.deadline(),
                &cargo_credit,
                &self.tool_context.session_id,
            );
            let prepared_request = optional_timeout(
                request_deadline,
                prepare_request_round(
                    self,
                    &mut messages,
                    &mut auto_compact,
                    &mut last_known_context_tokens,
                    &mut pressure,
                    reasoning_encrypted.clone(),
                    turn,
                ),
            )
            .await
            .map_err(|_| {
                window.stall_error("while preparing an LLM request", turn.saturating_add(1))
            })?;
            // A completed preparation may carry a compaction summary, which is
            // model output; without one it took no measurable time.
            archon_tools::subagent_activity::note();
            let inference_deadline = adjusted_deadline(
                window.deadline(),
                &cargo_credit,
                &self.tool_context.session_id,
            );
            let window_ref = &window;
            let inference = async {
                optional_timeout(
                    inference_deadline,
                    collect_stream_round(
                        self,
                        &mut messages,
                        &mut auto_compact,
                        (
                            &mut recovery_ladder,
                            &mut emergency_projection_pending,
                            &mut reactive_rate_limit_retried,
                            &mut last_known_context_tokens,
                        ),
                        prepared_request.template,
                        (
                            prepared_request.request_body_bytes,
                            prepared_request.large_retry_body_bytes,
                        ),
                        &prepared_request.telemetry,
                    ),
                )
                .await
                .map_err(|_| {
                    window_ref.stall_error("during LLM inference", turn.saturating_add(1))
                })?
            };
            let stream =
                await_cancellable_inference(inference, self.tool_context.cancel_parent.as_ref())
                    .await?;
            if stream.retry_after_compact {
                continue;
            }
            reasoning_encrypted = stream.reasoning_encrypted;
            recovery_ladder = crate::agent::autocompact::RecoveryLadder::default();
            emergency_projection_pending = false;
            reactive_rate_limit_retried = false;
            auto_compact.on_ordinary_success();
            cumulative_billable_tokens += stream.context_input_tokens;
            last_known_context_tokens = stream.context_input_tokens;
            tracing::trace!(cumulative_billable_tokens, "subagent billable input tokens");

            // If no tool calls, subagent is done — return accumulated text
            if stream.pending_tools.is_empty() {
                window.observe_turn(turn.saturating_add(1), &stream.text_content, &[], &[]);
                if crate::subagent::is_text_tool_call_only(&stream.text_content) {
                    tracing::warn!(
                        turn = turn.saturating_add(1),
                        "subagent wrote a tool call as text; requesting a tool-interface call or final answer"
                    );
                    let answer = serde_json::json!({
                        "role": "assistant",
                        "content": stream.text_content,
                    });
                    self.record_transcript(&answer);
                    messages.push(answer);
                    let feedback = serde_json::json!({
                        "role": "user",
                        "content": "Your last message wrote a tool call as text. Text tool calls are not run. Call the tool through the tool interface, or give your final answer.",
                    });
                    self.record_transcript(&feedback);
                    messages.push(feedback);
                    continue;
                }
                if let Some(landing) = &self.tool_context.audit_landing {
                    let parsed = serde_json::from_str::<serde_json::Value>(&stream.text_content);
                    let compact = parsed.as_ref().ok().and_then(|v| {
                        if landing.tool_name() == "land-audit-record" {
                            v.pointer("/data/repository_audit")
                        } else {
                            v.get("data")
                                .filter(|d| d.get("records_landed").is_some())
                                .or(Some(v))
                        }
                    });
                    if let Some(value) = compact.filter(|v| v.get("records_landed").is_some())
                        && let Err(error) = landing.complete(value)
                    {
                        if incomplete_audit_replies >= 2 {
                            anyhow::bail!("incomplete landed artifact: {error}");
                        }
                        incomplete_audit_replies += 1;
                        let answer =
                            serde_json::json!({"role":"assistant","content":stream.text_content});
                        self.record_transcript(&answer);
                        messages.push(answer);
                        let feedback = serde_json::json!({"role":"user","content":format!("Landed artifact incomplete: {error}. {}",landing.hint().unwrap_or_default())});
                        self.record_transcript(&feedback);
                        messages.push(feedback);
                        continue;
                    }
                }
                // Record final assistant text to transcript (AGT-024)
                if !stream.text_content.is_empty() {
                    self.record_transcript(&serde_json::json!({
                        "role": "assistant",
                        "content": stream.text_content,
                    }));
                }
                self.emit_activity_stream("final", "subagent turn complete", None, false);
                return Ok(stream.text_content);
            }

            let round_cancel = self
                .tool_context
                .cancel_parent
                .as_ref()
                .map(tokio_util::sync::CancellationToken::child_token)
                .unwrap_or_default();
            let activity_text = stream.text_content.clone();
            let activity_tools = stream.pending_tools.clone();
            // A tool round in flight is activity for the host's inactivity
            // bound for as long as it runs; its end is activity too.
            let round_activity = progress_stop::RoundActivity::of(
                stream.pending_tools.iter().map(|tool| tool.name.as_str()),
            );
            let activity = archon_tools::subagent_activity::tool_round();
            let (round_end, refused_calls) = await_tool_round(
                replay_tool_round(
                    self,
                    &mut messages,
                    stream.text_content,
                    stream.thinking_blocks,
                    stream.pending_tools,
                    round_cancel.clone(),
                ),
                round_cancel,
                &self.tool_context.session_id,
                window.deadline(),
                &cargo_credit,
            )
            .await;
            drop(activity);
            window.extend(cargo_credit.bank(&self.tool_context.session_id));
            // Issue-136: a cancelled session (a run paused or cancelled, its
            // call dropped) ends its tool round at once rather than at the
            // round's natural end; the tools' own trees are reaped on drop.
            if round_end == RoundEnd::Cancelled {
                anyhow::bail!("Subagent cancelled during tool round at turn {turn}");
            }
            if round_end == RoundEnd::TimedOut {
                return Err(window.stall_error("during a tool round", turn.saturating_add(1)));
            }
            // A novel request earns progress only when its result was admitted.
            // New assistant text remains independent progress.
            window.observe_turn(
                turn.saturating_add(1),
                &activity_text,
                &activity_tools,
                &refused_calls,
            );
            // The workflow read guard has refused this round terminally
            // (Issue-54): the agent thrashed past the read wall without
            // writing. Its next turn could only be more of the same, so the
            // session ends here, as a failure carrying the guard's own text,
            // which the write layer treats as a host interruption.
            if let Some(reason) = self
                .tool_context
                .workflow_read_guard
                .as_ref()
                .and_then(|guard| guard.terminal_failure())
            {
                self.emit_activity_stream("error", reason.clone(), None, true);
                anyhow::bail!("{reason}");
            }
            // Issue-213 C2d: still repeating after the reminder, tree unchanged.
            if let Some(reason) = progress_stop
                .after_round(&self.tool_context, round_activity)
                .await
            {
                self.emit_activity_stream("error", reason.clone(), None, true);
                anyhow::bail!("{reason}");
            }
        }

        self.emit_activity_stream(
            "error",
            format!("Subagent reached max turns ({})", self.max_turns),
            None,
            true,
        );
        anyhow::bail!("Subagent reached max turns ({})", self.max_turns)
    }
}

fn adjusted_deadline(
    deadline: Option<tokio::time::Instant>,
    credit: &CargoCredit,
    session: &str,
) -> Option<tokio::time::Instant> {
    deadline.map(|deadline| deadline + credit.live(session))
}

async fn optional_timeout<T>(
    deadline: Option<tokio::time::Instant>,
    work: impl std::future::Future<Output = T>,
) -> Result<T, tokio::time::error::Elapsed> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, work).await,
        None => Ok(work.await),
    }
}

async fn await_cancellable_inference<F, T>(
    inference: F,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    let Some(cancel) = cancel else {
        return inference.await;
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("Subagent cancelled during LLM inference"),
        result = inference => result,
    }
}

/// How a tool round ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundEnd {
    Finished,
    /// The session's no-progress window ran out.
    TimedOut,
    /// The session was cancelled from above while the round ran.
    Cancelled,
}

/// Grace a cut round's tools get to wind down before they are dropped.
const ROUND_CLEANUP: Duration = Duration::from_secs(2);

async fn await_tool_round<F>(
    future: F,
    round_cancel: tokio_util::sync::CancellationToken,
    session_id: &str,
    deadline: Option<tokio::time::Instant>,
    credit: &CargoCredit,
) -> (RoundEnd, Vec<bool>)
where
    F: std::future::Future<Output = Vec<bool>>,
{
    tokio::pin!(future);
    loop {
        let adjusted = deadline.map(|deadline| deadline + credit.live(session_id));
        let expiry = async {
            match adjusted {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            biased;
            _ = expiry => {
                let Some(deadline) = deadline else { continue };
                if tokio::time::Instant::now() < deadline + credit.live(session_id) {
                    continue;
                }
                round_cancel.cancel();
                let _ = tokio::time::timeout(ROUND_CLEANUP, &mut future).await;
                return (RoundEnd::TimedOut, Vec::new());
            }
            result = &mut future => return (RoundEnd::Finished, result),
            // Issue-136: the round's token is the session's child, so a
            // cancelled session is seen here; tools that watch the token end
            // themselves, and the rest are dropped after the grace.
            _ = round_cancel.cancelled() => {
                let _ = tokio::time::timeout(ROUND_CLEANUP, &mut future).await;
                return (RoundEnd::Cancelled, Vec::new());
            }
        }
    }
}

fn summarize_tool_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.chars().count() <= 500 {
        return trimmed.to_string();
    }
    let mut summary: String = trimmed.chars().take(500).collect();
    summary.push_str("...");
    summary
}
