use super::*;

mod await_round;
mod cargo_credit;
mod completion_usage;
mod context_fit;
#[cfg(test)]
mod context_fit_tests;
mod flatten_history;
mod landed_reply;
mod message_history;
mod progress_novelty;
mod progress_stop;
mod progress_window;
#[cfg(test)]
mod progress_window_tests;
mod provider_open;
mod request_round;
mod request_round_pressure;
pub(crate) mod stream_idle_window;
mod stream_resend_budget;
mod stream_round;
mod tool_helpers;
mod tool_round;

#[cfg(test)]
#[path = "runtime/read_range_history_recovery_tests.rs"]
mod read_range_history_recovery_tests;

use await_round::{RoundEnd, await_cancellable_inference, await_tool_round};
use cargo_credit::CargoCredit;
use message_history::MessageHistory;
use progress_window::ProgressWindow;
use provider_open::{adjusted_deadline, optional_timeout};
use request_round::{PressureState, prepare_request_round};
use stream_round::collect_stream_round;
use tool_round::replay_tool_round;

impl SubagentRunner {
    /// Run the subagent loop with the given initial prompt.
    /// Returns the accumulated text output from the final turn.
    pub async fn run(&self, initial_prompt: &str) -> anyhow::Result<String> {
        Ok(self.run_with_completion(initial_prompt).await?.content)
    }

    pub async fn run_with_completion(
        &self,
        initial_prompt: &str,
    ) -> anyhow::Result<super::SubagentCompletion> {
        // The session has started: from here, and not while it was queued for
        // a subagent slot, silence counts against the host's inactivity bound.
        archon_tools::subagent_activity::note();
        let mut messages = provider_open::initial_history(self, initial_prompt);

        // Issue 288: the session's limit is a no-progress window, never a
        // total. It renews only on novel activity (see `progress_window`).
        let timeout_secs = match archon_tools::host_timeout::current() {
            Some(archon_tools::host_timeout::HostTimeout::Unlimited) => None,
            Some(archon_tools::host_timeout::HostTimeout::Finite(seconds)) => Some(seconds),
            None => Some(self.timeout_secs),
        };
        // Workflow hosts own a dynamic session clock that excludes classified
        // outages. A second static deadline would cut before a fault returns.
        let mut window = ProgressWindow::open(if archon_tools::provider_recovery::enabled() {
            None
        } else {
            timeout_secs
        })?;
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
        let mut inspection_tools_withdrawn = false;

        for turn in 0..self.max_turns {
            // Issue-213 C5: the turn an interrupted call's record names.
            archon_tools::session_progress::note_turn(&progress_agent, turn.saturating_add(1));
            // The no-progress window ran out between turns. Its text names
            // the window and the last novel activity, so a reader can tell a
            // looping session from a slow one.
            if window.expired() {
                return self.complete_landed_or_error(
                    window.stall_error("between turns", turn.saturating_add(1)),
                );
            }

            // Check for graceful shutdown request
            if self
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return self
                    .retained_landed_completion()
                    .map(|content| {
                        completion_usage::from_stream(
                            content,
                            Some("shutdown".into()),
                            self.progress.as_ref(),
                        )
                    })
                    .map(Ok)
                    .unwrap_or_else(|| Ok(completion_usage::shutdown()));
            }

            if !inspection_tools_withdrawn
                && let Some(guard) = self
                    .tool_context
                    .workflow_read_guard
                    .as_ref()
                    .filter(|guard| guard.inspection_withdrawn_for(&self.tool_context))
            {
                let mut note =
                    withdrawn_note(guard.withdrawal_reason_for(&self.tool_context).as_deref());
                if let Some(landing) = &self.tool_context.audit_landing {
                    note.push_str(&format!("\n{}", landing.hint().unwrap_or_default()));
                }
                let message = flatten_history::flatten_history(
                    messages.as_slice(),
                    initial_prompt,
                    &note,
                    request_round::read_ceiling_fit_budget(self),
                );
                self.record_transcript(&message);
                messages.replace_for_agent(vec![message], &self.tool_context);
                inspection_tools_withdrawn = true;
            }

            if let Some(landing) = &self.tool_context.audit_landing {
                match landing.progress_message() {
                    Ok(Some(text)) => {
                        let message = serde_json::json!({"role":"user","content":text});
                        self.record_transcript(&message);
                        messages.push(message);
                    }
                    Ok(None) => {}
                    Err(error) => return self.complete_landed_or_error(anyhow::Error::msg(error)),
                }
            }
            let request_deadline = adjusted_deadline(
                window.deadline(),
                &cargo_credit,
                &self.tool_context.session_id,
            );
            let prepared_request = match Box::pin(provider_open::prepare(
                self,
                request_deadline,
                &window,
                &mut messages,
                &mut auto_compact,
                &mut last_known_context_tokens,
                &mut pressure,
                reasoning_encrypted.clone(),
                turn,
            ))
            .await
            {
                Ok(request) => request,
                Err(error)
                    if error
                        .downcast_ref::<progress_window::WindowExpired>()
                        .is_some() =>
                {
                    return self.complete_landed_or_error(error);
                }
                Err(error) => return Err(error),
            };
            let offered_tool_names = prepared_request
                .template
                .tools
                .iter()
                .filter_map(|definition| {
                    definition
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>();
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
                    Box::pin(collect_stream_round(
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
                    )),
                )
                .await
                .map_err(|_| {
                    window_ref.stall_error("during LLM inference", turn.saturating_add(1))
                })?
            };
            let stream = match await_cancellable_inference(
                inference,
                self.tool_context.cancel_parent.as_ref(),
            )
            .await
            {
                Ok(stream) => stream,
                Err(error)
                    if error
                        .downcast_ref::<progress_window::WindowExpired>()
                        .is_some() =>
                {
                    return self.complete_landed_or_error(error);
                }
                Err(error)
                    if error
                        .downcast_ref::<archon_tools::audit_landing::EmptyOutputLimit>()
                        .is_some()
                        || error
                            .to_string()
                            .contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER) =>
                {
                    return self.complete_landed_or_error(error);
                }
                Err(error) => return Err(error),
            };
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
                if crate::subagent::starts_with_text_tool_call(&stream.text_content) {
                    let tool_name =
                        crate::subagent::extract_text_tool_call_name(&stream.text_content)
                            .expect("recognized text tool-call prefix has a name");
                    // Text-form tool calls are rejected and re-asked below;
                    // changing their arguments does not count as progress.
                    window.observe_turn(turn.saturating_add(1), "", &[], &[]);
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
                        "content": if inspection_tools_withdrawn {
                            format!(
                                "Inspection tools are no longer available in this call and text tool calls are not run; answer now with your deliverable from what you have read. {}",
                                archon_tools::workflow_read_guard::ANSWER_IN_TASK_FORMAT
                            )
                        } else if !offered_tool_names.iter().any(|name| name == &tool_name) {
                            if offered_tool_names.is_empty() {
                                "No tools are available in this call; answer now with your deliverable from what you have read.".to_owned()
                            } else {
                                format!(
                                    "Your last message began with a call to `{tool_name}` written as text; `{tool_name}` is not available in this call, so it was not run. The tools you can call are: {}. Call one of them through the tool interface, or, if none fits, answer now with your deliverable from what you have read.",
                                    offered_tool_names.join(", ")
                                )
                            }
                        } else {
                            "Your last message began with a tool call written as text; it was not run. If that message also contained your final answer, repeat the final answer now without the tool-call text. Otherwise call the tool through the tool interface.".to_owned()
                        },
                    });
                    self.record_transcript(&feedback);
                    messages.push(feedback);
                    continue;
                }
                window.observe_turn(turn.saturating_add(1), &stream.text_content, &[], &[]);
                let mut content = stream.text_content.clone();
                let mut stop_reason = stream.stop_reason.clone();
                match self
                    .judge_landed_reply(&stream.text_content, &mut incomplete_audit_replies)?
                {
                    landed_reply::LandedReply::Final => {}
                    landed_reply::LandedReply::Retry(feedback) => {
                        let answer =
                            serde_json::json!({"role":"assistant","content":stream.text_content});
                        self.record_transcript(&answer);
                        messages.push(answer);
                        let feedback = serde_json::json!({"role":"user","content":feedback});
                        self.record_transcript(&feedback);
                        messages.push(feedback);
                        continue;
                    }
                    // The host's completion is a whole final reply.
                    landed_reply::LandedReply::Complete(completion) => {
                        content = completion;
                        stop_reason = Some("end_turn".to_owned());
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
                return Ok(completion_usage::from_stream(
                    content,
                    stop_reason,
                    self.progress.as_ref(),
                ));
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
            let (round_end, observed_calls) = await_tool_round(
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
                return self.complete_landed_or_error(
                    window.stall_error("during a tool round", turn.saturating_add(1)),
                );
            }
            // A novel request earns progress only when its result was admitted.
            // New assistant text remains independent progress.
            window.observe_turn(
                turn.saturating_add(1),
                &activity_text,
                &activity_tools,
                &observed_calls,
            );
            // The workflow read guard has refused this round terminally
            // (Issue-54): the agent thrashed past the read wall without
            // writing. Its next turn could only be more of the same, so the
            // invocation ends here, as a failure carrying the guard's own text,
            // which the write layer treats as a host interruption.
            if let Some(reason) = self
                .tool_context
                .workflow_read_guard
                .as_ref()
                .and_then(|guard| guard.terminal_failure_for(&self.tool_context))
            {
                if let Some(guard) = &self.tool_context.workflow_read_guard {
                    guard.charge_terminal_to_parent(&self.tool_context);
                }
                self.emit_activity_stream("error", reason.clone(), None, true);
                return self.complete_landed_or_error(anyhow::anyhow!(reason));
            }
            // Issue-213 C2d: still repeating after the reminder, tree unchanged.
            if let Some(reason) = progress_stop
                .after_round(&self.tool_context, round_activity)
                .await
            {
                // Issue-396: a read-only session's deliverable is its final
                // message, so the first stall withdraws inspection and asks
                // for it; the session fails only when it stalls again.
                if self
                    .tool_context
                    .workflow_read_guard
                    .as_ref()
                    .is_some_and(|guard| {
                        guard.withdraw_inspection_for(&self.tool_context, STALL_WITHDRAWAL_REASON)
                    })
                {
                    tracing::warn!(
                        turn = turn.saturating_add(1),
                        "read-only session kept getting the same results; inspection withdrawn and the deliverable requested"
                    );
                    continue;
                }
                // Issue-550: retained records the host can complete from end it instead.
                if let Some(completion) = self.retained_landed_completion() {
                    self.emit_activity_stream("final", reason, None, false);
                    // The host's completion is a whole final reply.
                    let stop = Some("end_turn".to_owned());
                    return Ok(completion_usage::from_stream(
                        completion,
                        stop,
                        self.progress.as_ref(),
                    ));
                }
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
        self.complete_landed_or_error(anyhow::anyhow!(
            "Subagent reached max turns ({})",
            self.max_turns
        ))
    }
}

/// Why a read-only session's inspection is withdrawn at its first stall.
const STALL_WITHDRAWAL_REASON: &str =
    "Your recent tool calls kept returning the same results, and nothing changed between them.";

/// The instruction that replaces the history when inspection is withdrawn.
fn withdrawn_note(reason: Option<&str>) -> String {
    format!(
        "{}Inspection tools are no longer available in this call; answer now with your deliverable from what you have read. {}",
        reason
            .map(|reason| format!("{reason} "))
            .unwrap_or_default(),
        archon_tools::workflow_read_guard::ANSWER_IN_TASK_FORMAT
    )
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
