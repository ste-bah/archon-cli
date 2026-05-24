use super::*;

pub(super) struct PreparedRequest {
    pub request: LlmRequest,
    pub request_body_bytes: usize,
    pub large_retry_body_bytes: usize,
    pub telemetry: crate::agent::autocompact::CompactionTelemetry,
}

pub(super) async fn prepare_request_round(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    proactive_pressure_attempted: &mut bool,
    reasoning_encrypted: Option<String>,
) -> PreparedRequest {
    let telemetry = build_compaction_telemetry(runner);
    maybe_compact_for_context_window(
        runner,
        messages,
        auto_compact,
        last_known_context_tokens,
        &telemetry,
    )
    .await;

    let mut request = build_llm_request(runner, messages, reasoning_encrypted).await;
    let mut request_body_bytes = crate::agent::autocompact::request_body_bytes(&request);
    let large_retry_body_bytes =
        crate::agent::autocompact::large_request_retry_body_bytes(&runner.agent_config.context);

    maybe_compact_for_request_pressure(
        runner,
        messages,
        auto_compact,
        last_known_context_tokens,
        proactive_pressure_attempted,
        &mut request,
        &mut request_body_bytes,
        &telemetry,
    )
    .await;

    PreparedRequest {
        request,
        request_body_bytes,
        large_retry_body_bytes,
        telemetry,
    }
}

fn build_compaction_telemetry(
    runner: &SubagentRunner,
) -> crate::agent::autocompact::CompactionTelemetry {
    let resolved_window = archon_llm::context_window::resolve_context_window_for_work_dir(
        &runner.model,
        runner
            .agent_config
            .context
            .context_window_override
            .or_else(|| runner.agent_config.context.max_tokens.map(u64::from)),
        Some(runner.provider.as_ref()),
        Some(&runner.agent_config.working_dir),
    );
    crate::agent::autocompact::CompactionTelemetry {
        provider_family: runner.provider.compaction_policy().provider_family.label(),
        wire_shape: runner.provider.compaction_policy().wire_shape.label(),
        native_context_window: resolved_window.context_window,
        runtime_context_budget: resolved_window
            .runtime_context_budget
            .unwrap_or(resolved_window.context_window),
        context_source: resolved_window.source.label(),
        compaction_backend: runner.provider.compaction_policy().backend.label(),
    }
}

async fn maybe_compact_for_context_window(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
) {
    let effective_window = telemetry
        .runtime_context_budget
        .saturating_sub(runner.agent_config.context.output_reserve_tokens);
    let threshold = (runner.agent_config.context.compact_threshold
        - runner.agent_config.context.preflight_safety_margin)
        .max(0.0);
    let trigger_tokens = current_trigger_tokens(messages, *last_known_context_tokens);
    let Some(action) = crate::agent::evaluate_compaction(
        trigger_tokens,
        effective_window,
        auto_compact,
        threshold,
    ) else {
        return;
    };

    tracing::info!(
        compaction.reason = "context_window_threshold",
        provider_family = telemetry.provider_family,
        wire_shape = telemetry.wire_shape,
        native_context_window = telemetry.native_context_window,
        runtime_context_budget = telemetry.runtime_context_budget,
        context_source = telemetry.context_source,
        compaction_backend = telemetry.compaction_backend,
        scope = "subagent",
        force = false,
        consecutive_failures = auto_compact.consecutive_failures,
        "subagent auto-compaction attempt started"
    );
    compact_proactively(
        runner,
        messages,
        auto_compact,
        last_known_context_tokens,
        telemetry,
        action,
        "proactive subagent compaction failed; continuing turn",
    )
    .await;
}

async fn build_llm_request(
    runner: &SubagentRunner,
    messages: &[serde_json::Value],
    reasoning_encrypted: Option<String>,
) -> LlmRequest {
    let (max_tokens, thinking, speed) =
        runner.agent_config.build_base_request_fields(&runner.model);
    LlmRequest {
        model: runner.model.clone(),
        max_tokens,
        system: build_system_messages(runner, messages),
        messages: messages.to_vec(),
        tools: runner.tool_definitions.clone(),
        thinking,
        speed,
        effort: resolve_effort(runner).await,
        extra: serde_json::Value::Null,
        request_origin: Some("subagent".into()),
        reasoning_encrypted,
    }
}

fn build_system_messages(
    runner: &SubagentRunner,
    messages: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut system = Vec::new();
    let first_user_message = messages
        .first()
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .unwrap_or("");
    if let Some(billing) = runner.identity.billing_header(first_user_message) {
        system.push(serde_json::json!({
            "type": "text",
            "text": billing,
            "cache_control": { "type": "ephemeral" }
        }));
    }
    system.push(serde_json::json!({
        "type": "text",
        "text": &runner.system_prompt,
    }));
    if let Some(ref reminder) = runner.critical_system_reminder {
        system.push(serde_json::json!({
            "type": "text",
            "text": format!("<system-reminder>{reminder}</system-reminder>"),
        }));
    }
    system
}

async fn resolve_effort(runner: &SubagentRunner) -> Option<String> {
    if runner.effort.is_some() {
        return runner.effort.clone();
    }
    let level = runner.agent_config.effort_level.lock().await;
    match *level {
        archon_llm::effort::EffortLevel::High => None,
        other => Some(other.to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn maybe_compact_for_request_pressure(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    proactive_pressure_attempted: &mut bool,
    request: &mut LlmRequest,
    request_body_bytes: &mut usize,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
) {
    let trigger_tokens = current_trigger_tokens(messages, *last_known_context_tokens);
    let token_pressure = runner
        .agent_config
        .context
        .rate_limit_pressure_tokens
        .is_some_and(|threshold| trigger_tokens >= threshold);
    let body_pressure = runner
        .agent_config
        .context
        .rate_limit_pressure_body_bytes
        .is_some_and(|threshold| *request_body_bytes as u64 >= threshold);
    if *proactive_pressure_attempted
        || !(token_pressure || body_pressure)
        || !auto_compact.should_attempt()
    {
        return;
    }
    *proactive_pressure_attempted = true;
    let reason = pressure_reason(token_pressure, body_pressure);
    tracing::info!(
        compaction.reason = reason,
        trigger_tokens,
        trigger_body_bytes = *request_body_bytes,
        context_window = telemetry.runtime_context_budget,
        provider_family = telemetry.provider_family,
        wire_shape = telemetry.wire_shape,
        native_context_window = telemetry.native_context_window,
        runtime_context_budget = telemetry.runtime_context_budget,
        context_source = telemetry.context_source,
        compaction_backend = telemetry.compaction_backend,
        scope = "subagent",
        force = false,
        consecutive_failures = auto_compact.consecutive_failures,
        "subagent request pressure threshold reached; attempting proactive compaction"
    );
    compact_proactively(
        runner,
        messages,
        auto_compact,
        last_known_context_tokens,
        telemetry,
        crate::agent::CompactAction::Full,
        "subagent request-pressure compaction failed; continuing turn",
    )
    .await;
    request.messages = messages.clone();
    *request_body_bytes = crate::agent::autocompact::request_body_bytes(request);
}

fn current_trigger_tokens(messages: &[serde_json::Value], last_known_context_tokens: u64) -> u64 {
    if last_known_context_tokens > 0 {
        last_known_context_tokens
    } else {
        crate::agent::autocompact::trigger_tokens(messages)
    }
}

fn pressure_reason(token_pressure: bool, body_pressure: bool) -> &'static str {
    match (token_pressure, body_pressure) {
        (true, true) => "request_pressure_tokens_and_bytes",
        (true, false) => "request_pressure_tokens",
        (false, true) => "request_pressure_bytes",
        (false, false) => unreachable!(),
    }
}

async fn compact_proactively(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
    action: crate::agent::CompactAction,
    failure_message: &str,
) {
    auto_compact.compact_in_flight = true;
    match crate::agent::autocompact::compact_json_messages_with_provider(
        runner.provider.as_ref(),
        &runner.model,
        messages,
        action,
        false,
    )
    .await
    {
        Ok((
            crate::agent::autocompact::CompactionOutcome::Compacted {
                after_estimated_tokens,
                ..
            },
            compacted,
        )) => {
            *messages = compacted;
            *last_known_context_tokens = 0;
            auto_compact.on_success(after_estimated_tokens);
        }
        Ok((crate::agent::autocompact::CompactionOutcome::Skipped { .. }, _)) => {
            auto_compact.on_cancel();
        }
        Err(crate::agent::autocompact::CompactionError::Cancelled) => {
            auto_compact.on_cancel();
            tracing::debug!(
                compaction.outcome = "cancelled",
                provider_family = telemetry.provider_family,
                wire_shape = telemetry.wire_shape,
                native_context_window = telemetry.native_context_window,
                runtime_context_budget = telemetry.runtime_context_budget,
                context_source = telemetry.context_source,
                compaction_backend = telemetry.compaction_backend,
                actor = %runner.activity_actor_id.as_deref().unwrap_or("subagent"),
                "proactive subagent compaction cancelled"
            );
        }
        Err(error) => {
            auto_compact.on_real_failure();
            tracing::warn!(
                compaction.outcome = "auto_failed",
                provider_family = telemetry.provider_family,
                wire_shape = telemetry.wire_shape,
                native_context_window = telemetry.native_context_window,
                runtime_context_budget = telemetry.runtime_context_budget,
                context_source = telemetry.context_source,
                compaction_backend = telemetry.compaction_backend,
                actor = %runner.activity_actor_id.as_deref().unwrap_or("subagent"),
                consecutive_failures = auto_compact.consecutive_failures,
                breaker_tripped = auto_compact.disabled,
                error = %error,
                "{failure_message}",
            );
        }
    }
}
