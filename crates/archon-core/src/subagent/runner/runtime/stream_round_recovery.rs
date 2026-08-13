use super::*;

pub(super) fn projected_request(
    runner: &SubagentRunner,
    messages: &[serde_json::Value],
    template: &LlmRequest,
) -> LlmRequest {
    let projected = crate::agent::tool_result_context::project_messages_for_request(
        messages,
        runner.agent_config.context.preserve_recent_turns,
    );
    request_with_messages(runner, template, projected)
}

/// Open the round's stream, walking the #103 recovery ladder on request
/// pressure.
///
/// Returns the receiver together with the request that actually opened it, so
/// the caller can classify a mid-stream failure against the bytes that were
/// really sent. The request arrives owned and becomes the first attempt
/// directly — it used to be cloned on the way in and then cloned again for
/// every `stream` call, two full message-array deep clones per round on the
/// quiet path (#171 part 2). One clone per call remains because `stream`
/// consumes its request and the error arms need the failed one to classify
/// and to measure.
#[allow(clippy::too_many_arguments)]
pub(super) async fn open_stream_with_retries(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    recovery_ladder: &mut crate::agent::autocompact::RecoveryLadder,
    reactive_rate_limit_retried: &mut bool,
    last_known_context_tokens: &mut u64,
    request: LlmRequest,
    request_body_bytes: usize,
    large_retry_body_bytes: usize,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
) -> anyhow::Result<(tokio::sync::mpsc::Receiver<StreamEvent>, LlmRequest)> {
    let mut attempt_request = request;
    loop {
        match runner.provider.stream(attempt_request.clone()).await {
            Ok(rx) => return Ok((rx, attempt_request)),
            Err(error)
                if crate::agent::autocompact::request_pressure_kind_for_request(
                    &error,
                    &attempt_request,
                )
                .is_some() =>
            {
                let failed_body_bytes =
                    crate::agent::autocompact::request_body_bytes(&attempt_request);
                let classification = crate::agent::autocompact::request_pressure_kind_for_request(
                    &error,
                    &attempt_request,
                )
                .expect("guarded request pressure classification");
                let Some(mut tier) = recovery_ladder.next(classification) else {
                    anyhow::bail!("request pressure recovery exhausted after two bounded retries");
                };
                attempt_request = match tier {
                    crate::agent::autocompact::RecoveryTier::FullCompaction => {
                        match compact_messages_for_retry(
                            runner,
                            messages,
                            auto_compact,
                            last_known_context_tokens,
                        )
                        .await
                        {
                            Ok(()) => {
                                projected_request(runner, messages.as_slice(), &attempt_request)
                            }
                            Err(crate::agent::autocompact::CompactionError::NoSafeBoundary) => {
                                tier = recovery_ladder.next(classification).ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "request pressure recovery exhausted after two bounded retries"
                                    )
                                })?;
                                debug_assert_eq!(
                                    tier,
                                    crate::agent::autocompact::RecoveryTier::EmergencyProjection
                                );
                                emergency_projected_request(
                                    runner,
                                    messages.as_slice(),
                                    &attempt_request,
                                )
                            }
                            Err(error) => {
                                return Err(anyhow::anyhow!(
                                    "reactive subagent compaction failed: {error}"
                                ));
                            }
                        }
                    }
                    crate::agent::autocompact::RecoveryTier::EmergencyProjection => {
                        emergency_projected_request(runner, messages.as_slice(), &attempt_request)
                    }
                };
                log_recovery_retry(
                    runner,
                    classification,
                    tier,
                    failed_body_bytes,
                    &attempt_request,
                    "subagent",
                );
            }
            Err(error)
                if crate::agent::autocompact::is_rate_limited_error(&error)
                    && !*reactive_rate_limit_retried
                    && request_body_bytes >= large_retry_body_bytes =>
            {
                *reactive_rate_limit_retried = true;
                tracing::warn!(
                    compaction.reason = "rate_limit_large_request",
                    trigger_body_bytes = request_body_bytes,
                    threshold_body_bytes = large_retry_body_bytes,
                    provider_family = telemetry.provider_family,
                    wire_shape = telemetry.wire_shape,
                    native_context_window = telemetry.native_context_window,
                    runtime_context_budget = telemetry.runtime_context_budget,
                    context_source = telemetry.context_source,
                    compaction_backend = telemetry.compaction_backend,
                    scope = "subagent",
                    force = true,
                    "rate-limited subagent request is large; compacting before one retry"
                );
                compact_messages_for_retry(
                    runner,
                    messages,
                    auto_compact,
                    last_known_context_tokens,
                )
                .await
                .map_err(|error| {
                    anyhow::anyhow!("rate-limit subagent compaction failed: {error}")
                })?;
                attempt_request = projected_request(runner, messages.as_slice(), &attempt_request);
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        }
    }
}

pub(super) fn emergency_projected_request(
    runner: &SubagentRunner,
    messages: &[serde_json::Value],
    request: &LlmRequest,
) -> LlmRequest {
    let projected = crate::agent::tool_result_context::project_messages_for_emergency_retry(
        messages,
        crate::agent::tool_result_context::emergency_tool_result_bytes(
            crate::agent::tool_result_context::resolved_max_tool_result_bytes(
                runner.agent_config.context.max_tool_result_bytes,
                runner.provider.as_ref(),
            ),
        ),
    );
    request_with_messages(runner, request, projected)
}

/// Rebuild a request around a fresh message array.
///
/// The fields are copied one by one rather than through `..template.clone()`:
/// functional update syntax clones the whole base first and then drops the
/// field being overridden, so the old shape deep-cloned the entire message
/// array on every rebuild only to throw it away (#171 part 2). Everything
/// copied here is small — the tool list is an `Arc` (#171 part 3) and the
/// system blocks are a handful of KB.
fn request_with_messages(
    runner: &SubagentRunner,
    template: &LlmRequest,
    messages: Vec<serde_json::Value>,
) -> LlmRequest {
    let mut projected = LlmRequest {
        model: template.model.clone(),
        max_tokens: template.max_tokens,
        system: template.system.clone(),
        messages,
        tools: archon_llm::provider::SharedTools::clone(&template.tools),
        thinking: template.thinking.clone(),
        speed: template.speed.clone(),
        effort: template.effort.clone(),
        extra: template.extra.clone(),
        request_origin: template.request_origin.clone(),
        reasoning_encrypted: template.reasoning_encrypted.clone(),
    };
    crate::agent::request_cache::apply_conversation_cache(
        &mut projected,
        runner.provider.as_ref(),
        &runner.agent_config.context.prompt_cache_strategy,
        runner.agent_config.context.prompt_cache,
        runner.agent_config.context.prompt_cache_conversation,
        &runner.agent_config.context.prompt_cache_mode,
        &runner.agent_config.context.prompt_cache_ttl,
        &runner.agent_config.context.prompt_cache_models,
    );
    projected
}

fn log_recovery_retry(
    runner: &SubagentRunner,
    classification: crate::agent::autocompact::RequestPressureKind,
    tier: crate::agent::autocompact::RecoveryTier,
    before_body_bytes: usize,
    request: &LlmRequest,
    scope: &str,
) {
    let recovery = crate::agent::autocompact::RecoveryTelemetry::new(
        classification,
        tier,
        before_body_bytes,
        crate::agent::autocompact::request_body_bytes(request),
    )
    .with_cooldown_secs(None);
    let message =
        serde_json::to_string(&recovery).unwrap_or_else(|_| "request pressure recovery".into());
    if let Some(sink) = &runner.agent_config.activity_sink {
        sink.emit(
            archon_observability::AgentActivityEvent::new(
                runner.agent_config.session_id.clone(),
                archon_observability::AgentActivityKind::AgentRunning,
                archon_observability::AgentActivityStatus::Running,
                message,
            )
            .with_subagent_id(
                runner
                    .activity_actor_id
                    .clone()
                    .unwrap_or_else(|| "subagent".into()),
            )
            .with_provider_model(runner.provider.name(), runner.model.clone()),
        );
    }
    tracing::warn!(
        compaction.classification = ?recovery.classification,
        compaction.tier = ?recovery.tier,
        before_body_bytes = recovery.before_body_bytes,
        after_body_bytes = recovery.after_body_bytes,
        before_estimated_tokens = recovery.before_estimated_tokens,
        after_estimated_tokens = recovery.after_estimated_tokens,
        reduced = recovery.reduced,
        scope,
        "request pressure recovery retry"
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_stream_error(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    recovery_ladder: &mut crate::agent::autocompact::RecoveryLadder,
    emergency_projection_pending: &mut bool,
    reactive_rate_limit_retried: &mut bool,
    last_known_context_tokens: &mut u64,
    request_body_bytes: usize,
    large_retry_body_bytes: usize,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
    request: &LlmRequest,
    error_type: String,
    message: String,
) -> anyhow::Result<bool> {
    let err = crate::agent::autocompact::classify_stream_error(
        runner.provider.name(),
        &error_type,
        &message,
    );
    if let Some(classification) =
        crate::agent::autocompact::request_pressure_kind_for_request(&err, request)
    {
        let Some(mut tier) = recovery_ladder.next(classification) else {
            anyhow::bail!("request pressure recovery exhausted after two bounded retries");
        };
        // Recovery telemetry compares before/after body sizes, so both sides
        // must be measured the same way. The round's `request_body_bytes` is
        // now derived (#171 part 7); this is a rare error path, so the "before"
        // side is measured here rather than mixing an estimate with the
        // measured "after" and skewing the `reduced` verdict.
        let before_body_bytes = crate::agent::autocompact::request_body_bytes(request);
        match tier {
            crate::agent::autocompact::RecoveryTier::FullCompaction => {
                match compact_messages_for_retry(
                    runner,
                    messages,
                    auto_compact,
                    last_known_context_tokens,
                )
                .await
                {
                    Ok(()) => {
                        let retry_request = projected_request(runner, messages.as_slice(), request);
                        log_recovery_retry(
                            runner,
                            classification,
                            tier,
                            before_body_bytes,
                            &retry_request,
                            "subagent",
                        );
                    }
                    Err(crate::agent::autocompact::CompactionError::NoSafeBoundary) => {
                        tier = recovery_ladder.next(classification).ok_or_else(|| {
                            anyhow::anyhow!(
                                "request pressure recovery exhausted after two bounded retries"
                            )
                        })?;
                        debug_assert_eq!(
                            tier,
                            crate::agent::autocompact::RecoveryTier::EmergencyProjection
                        );
                        *emergency_projection_pending = true;
                        let retry_request =
                            emergency_projected_request(runner, messages.as_slice(), request);
                        log_recovery_retry(
                            runner,
                            classification,
                            tier,
                            before_body_bytes,
                            &retry_request,
                            "subagent",
                        );
                    }
                    Err(error) => {
                        return Err(anyhow::anyhow!(
                            "reactive subagent compaction failed: {error}"
                        ));
                    }
                }
            }
            crate::agent::autocompact::RecoveryTier::EmergencyProjection => {
                *emergency_projection_pending = true;
                let retry_request =
                    emergency_projected_request(runner, messages.as_slice(), request);
                log_recovery_retry(
                    runner,
                    classification,
                    tier,
                    before_body_bytes,
                    &retry_request,
                    "subagent",
                );
            }
        }
        return Ok(true);
    }
    if crate::agent::autocompact::is_rate_limited_error(&err)
        && !*reactive_rate_limit_retried
        && request_body_bytes >= large_retry_body_bytes
    {
        *reactive_rate_limit_retried = true;
        tracing::warn!(
            compaction.reason = "rate_limit_large_request_stream",
            trigger_body_bytes = request_body_bytes,
            threshold_body_bytes = large_retry_body_bytes,
            provider_family = telemetry.provider_family,
            wire_shape = telemetry.wire_shape,
            native_context_window = telemetry.native_context_window,
            runtime_context_budget = telemetry.runtime_context_budget,
            context_source = telemetry.context_source,
            compaction_backend = telemetry.compaction_backend,
            scope = "subagent",
            force = true,
            "rate-limited subagent stream is large; compacting before one retry"
        );
        compact_messages_for_retry(runner, messages, auto_compact, last_known_context_tokens)
            .await
            .map_err(|error| anyhow::anyhow!("rate-limit subagent compaction failed: {error}"))?;
        return Ok(true);
    }
    runner.emit_activity_stream("error", message, None, true);
    Err(anyhow::Error::new(err))
}

pub(super) async fn compact_messages_for_retry(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
) -> Result<(), crate::agent::autocompact::CompactionError> {
    let attribution = runner.agent_config.runtime_attribution_extra(
        "compaction",
        "subagent_reactive_compaction",
        None,
        None,
        None,
    );
    let result = crate::agent::autocompact::compact_json_messages_with_provider(
        runner.provider.as_ref(),
        &runner.model,
        messages.as_slice(),
        crate::agent::CompactAction::Full,
        true,
        attribution,
    )
    .await;
    let (outcome, compacted) = match result {
        Ok(result) => result,
        Err(error) => {
            auto_compact.on_failure(&error);
            return Err(error);
        }
    };
    messages.replace(compacted);
    let after_current_tokens = match outcome {
        crate::agent::autocompact::CompactionOutcome::Compacted {
            after_estimated_tokens,
            ..
        } => after_estimated_tokens,
        // `replace` already recomputed this; #171 part 1 removed the second
        // full pass, not the number.
        crate::agent::autocompact::CompactionOutcome::Skipped { .. } => messages.estimated_tokens(),
    };
    *last_known_context_tokens = 0;
    auto_compact.on_success(after_current_tokens);
    Ok(())
}
