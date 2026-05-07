use archon_tools::send_message::SendMessageRequest;

use super::tool_types::PreflightResult;
use super::*;

impl Agent {
    pub(super) async fn maybe_handle_send_message_result(
        &mut self,
        pre: &PreflightResult,
        result: ToolResult,
        active_model: &str,
    ) -> ToolResult {
        // CRIT-07 + AGT-026: Intercept SendMessage and route to target agent.
        // 4 delivery paths:
        //   A. Running in memory -> queue message
        //   B. Stopped in state, has transcript -> resume
        //   C. Evicted from state, transcript on disk -> resume
        //   D. No transcript -> error
        let result = if !result.is_error && pre.tool_name == "SendMessage" {
            match serde_json::from_str::<SendMessageRequest>(&result.content) {
                Ok(req) => match req.message_type.as_str() {
                    "text" => {
                        // AGT-026: Resolve target via name registry, then format validation
                        let (agent_id, is_running) = {
                            let mgr = self.subagent_manager.lock().await;
                            let resolved = if let Some(id) = mgr.resolve_name(&req.to) {
                                Some(id.to_string())
                            } else if archon_tools::send_message::is_valid_agent_id(&req.to) {
                                Some(req.to.clone())
                            } else {
                                None
                            };
                            let running = resolved
                                .as_ref()
                                .map(|id| mgr.is_running(id))
                                .unwrap_or(false);
                            (resolved, running)
                        };

                        match agent_id {
                            None => {
                                // Not in name registry, not a valid agent ID format
                                ToolResult::error(format!(
                                    "Unknown agent '{}' -- not in name registry and not a valid agent ID",
                                    req.to
                                ))
                            }
                            Some(agent_id) if is_running => {
                                // Path A: Agent is running — queue message for delivery
                                {
                                    let mut mgr = self.subagent_manager.lock().await;
                                    mgr.queue_pending_message(&agent_id, req.message.clone());
                                }
                                self.send_event(AgentEvent::MessageSent {
                                    target_agent_id: agent_id.clone(),
                                    message: req.message.clone(),
                                })
                                .await;
                                ToolResult::success(format!(
                                    "Message queued for delivery to {} at its next tool round.",
                                    req.to
                                ))
                            }
                            Some(agent_id) => {
                                // Path B+C: Agent not running — try to resume from transcript
                                let resume_ctx =
                                    crate::agents::transcript::AgentTranscriptStore::new(
                                        &self.config.session_id,
                                    )
                                    .and_then(|store| {
                                        crate::agents::transcript::load_resume_context(
                                            &store, &agent_id,
                                        )
                                    });

                                if let Some(ctx) = resume_ctx {
                                    tracing::info!(
                                        agent_id = %agent_id,
                                        agent_type = %ctx.agent_type,
                                        history_len = ctx.messages.len(),
                                        "Resuming agent from transcript"
                                    );
                                    let resume_request = archon_tools::agent_tool::SubagentRequest {
                                    prompt: req.message.clone(),
                                    model: None,
                                    allowed_tools: Vec::new(),
                                    max_turns: archon_tools::agent_tool::SubagentRequest::DEFAULT_MAX_TURNS,
                                    timeout_secs: archon_tools::agent_tool::SubagentRequest::DEFAULT_TIMEOUT_SECS,
                                    subagent_type: Some(ctx.agent_type),
                                    run_in_background: true,
                                    cwd: None,
                                    isolation: None,
                                };
                                    let resume_json =
                                        serde_json::to_string(&resume_request).unwrap_or_default();
                                    let resume_result = ToolResult {
                                        content: resume_json,
                                        is_error: false,
                                    };
                                    *self.pending_resume_messages.lock().await = Some(ctx.messages);
                                    self.send_event(AgentEvent::MessageSent {
                                        target_agent_id: agent_id.clone(),
                                        message: req.message.clone(),
                                    })
                                    .await;
                                    // TASK-AGS-105 Section 2f: route resume through
                                    // run_subagent (the AGT-025 auto-bg race still
                                    // applies) instead of the legacy
                                    // handle_subagent_result indirection.
                                    let _ = resume_result; // legacy stub, drop
                                    let resume_sid = agent_id.clone();
                                    let cancel = tokio_util::sync::CancellationToken::new();
                                    let resume_ctx = archon_tools::tool::ToolContext {
                                        working_dir: self.config.working_dir.clone(),
                                        session_id: self.config.session_id.clone(),
                                        mode: archon_tools::tool::AgentMode::Normal,
                                        extra_dirs: vec![],
                                        in_fork:
                                            crate::agents::built_in::is_in_fork_child_by_messages(
                                                &self.state.messages,
                                            ),
                                        nested: false,
                                        cancel_parent: self.config.cancel_token.clone(),
                                        sandbox: self.config.sandbox.clone(),
                                        activity_sink: self
                                            .provider_model_activity_sink(active_model),
                                    };
                                    match archon_tools::agent_tool::run_subagent(
                                        resume_sid,
                                        resume_request,
                                        cancel,
                                        resume_ctx,
                                    ).await {
                                        archon_tools::subagent_executor::SubagentOutcome::Completed(text) => ToolResult::success(text),
                                        archon_tools::subagent_executor::SubagentOutcome::Failed(err) => ToolResult::error(err),
                                        archon_tools::subagent_executor::SubagentOutcome::AutoBackgrounded => ToolResult::success(format!(
                                            "Subagent '{}' auto-backgrounded. Still running — use SendMessage to check status.",
                                            agent_id
                                        )),
                                        archon_tools::subagent_executor::SubagentOutcome::Cancelled => ToolResult::error("subagent cancelled"),
                                    }
                                } else {
                                    // Path D: No transcript found — error
                                    ToolResult::error(format!(
                                        "No transcript found for agent '{}'",
                                        req.to
                                    ))
                                }
                            }
                        }
                    }
                    "shutdown_request" => {
                        let mgr = self.subagent_manager.lock().await;
                        // Try by name first, then by raw ID
                        let target_id = mgr
                            .resolve_name(&req.to)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| req.to.clone());
                        if mgr.request_shutdown(&target_id) {
                            ToolResult::success(format!(
                                "Shutdown requested for agent '{}'",
                                req.to
                            ))
                        } else {
                            ToolResult::error(format!(
                                "Agent '{}' not found or not running",
                                req.to
                            ))
                        }
                    }
                    "shutdown_response" | "plan_approval_response" => {
                        // TASK-T2 (G2): Structured response message types.
                        // Build an XML envelope and deliver via the pending-message
                        // queue so the target agent can parse it on its next tool round.
                        let envelope = archon_tools::send_message::build_structured_envelope(&req);
                        let delivered = {
                            let mut mgr = self.subagent_manager.lock().await;
                            let target_id = mgr
                                .resolve_name(&req.to)
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| req.to.clone());
                            if !mgr.is_running(&target_id) {
                                None
                            } else {
                                mgr.queue_pending_message(&target_id, envelope);
                                Some(target_id)
                            }
                        };

                        match delivered {
                            Some(target_id) => {
                                // Guard has been dropped — safe to send event.
                                self.send_event(AgentEvent::MessageSent {
                                    target_agent_id: target_id,
                                    message: format!(
                                        "[{}] request_id={}",
                                        req.message_type,
                                        req.request_id.as_deref().unwrap_or("")
                                    ),
                                })
                                .await;
                                ToolResult::success(format!(
                                    "{} delivered to {}",
                                    req.message_type, req.to
                                ))
                            }
                            None => ToolResult::error(format!(
                                "Agent '{}' not running — cannot deliver structured response",
                                req.to
                            )),
                        }
                    }
                    other => ToolResult::error(format!("Unknown message_type: {}", other)),
                },
                Err(e) => ToolResult::error(format!("Failed to parse SendMessage result: {e}")),
            }
        } else {
            result
        };
        result
    }
}
