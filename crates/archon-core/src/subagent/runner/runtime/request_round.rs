use super::*;

pub(super) struct PreparedRequest {
    /// The round's request with everything settled except its message array.
    ///
    /// #171 part 2: the projection is the round's one O(N) pass over history,
    /// so it runs once, at the point the request is actually handed to the
    /// provider (`stream_round`), rather than here and then again — and once
    /// more when pressure compaction rewrote history — on the way there. The
    /// message array is left empty deliberately: everything downstream of this
    /// struct fills it from the live history, and `request_envelope_bytes`
    /// measures the request with an empty array anyway.
    pub template: LlmRequest,
    pub request_body_bytes: usize,
    pub large_retry_body_bytes: usize,
    pub telemetry: crate::agent::autocompact::CompactionTelemetry,
}

/// Cross-round pressure bookkeeping owned by the run loop.
#[derive(Default)]
pub(super) struct PressureState {
    /// Guards retry of a FAILED proactive compaction — see
    /// `request_round_pressure::proactive_rearmed`. `None` means armed.
    pub proactive_retry_watermark: Option<u64>,
    /// Serialized size of the request's fixed parts, measured on the first
    /// round and reused (#171 part 7).
    ///
    /// What it covers is stable for the run: the tool list is frozen for the
    /// session (#75 A3) and shared as an `Arc` (#171 part 3), the system
    /// prompt, stable workflow blocks and critical reminder are set before the
    /// run starts, and the billing header — the one system block derived from
    /// the conversation — is a fixed-length fingerprint, so a compaction that
    /// rewrites the first message does not change its size. The two fields
    /// that genuinely vary per round are the turn counter's digits in `extra`
    /// (a byte or two) and `reasoning_encrypted`, which is added back in
    /// per round rather than baked into this number.
    pub envelope_bytes: Option<usize>,
}

pub(super) async fn prepare_request_round(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    pressure: &mut PressureState,
    reasoning_encrypted: Option<String>,
    turn: u32,
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

    let template = build_llm_request(runner, messages.as_slice(), reasoning_encrypted, turn).await;
    // The envelope is measured with `reasoning_encrypted` absent on the first
    // round, so the current round's blob is added back on top of it.
    let envelope_bytes = pressure
        .envelope_bytes
        .get_or_insert_with(|| crate::agent::autocompact::request_envelope_bytes(&template))
        .saturating_add(template.reasoning_encrypted.as_ref().map_or(0, String::len));
    let mut request_body_bytes = crate::agent::autocompact::estimated_body_bytes(
        envelope_bytes,
        messages.estimated_tokens(),
    );
    log_request_size(runner, &template, request_body_bytes, messages);
    let large_retry_body_bytes =
        crate::agent::autocompact::large_request_retry_body_bytes(&runner.agent_config.context);

    maybe_compact_for_request_pressure(
        runner,
        messages,
        auto_compact,
        last_known_context_tokens,
        (pressure, envelope_bytes),
        &mut request_body_bytes,
        &telemetry,
    )
    .await;

    PreparedRequest {
        template,
        request_body_bytes,
        large_retry_body_bytes,
        telemetry,
    }
}

/// Preflight size line for the subagent request.
///
/// The main-agent path still gets this from `request_body_bytes`; here the
/// number is derived (#171 part 7), so it is reported as an estimate rather
/// than dressed up as a measurement.
fn log_request_size(
    runner: &SubagentRunner,
    request: &LlmRequest,
    request_body_bytes: usize,
    messages: &MessageHistory,
) {
    tracing::info!(
        target: "archon::context",
        request_origin = request.request_origin.as_deref().unwrap_or("subagent"),
        request_model = %request.model,
        request_body_bytes_estimated = request_body_bytes,
        request_approx_tokens = crate::agent::autocompact::approx_tokens_from_bytes(
            request_body_bytes
        ),
        request_message_count = messages.as_slice().len(),
        request_tool_count = runner.tool_definitions.len(),
        "llm request size preflight (estimated)"
    );
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
    messages: &mut MessageHistory,
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
    let _ = super::request_round_pressure::compact_proactively(
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
    turn: u32,
) -> LlmRequest {
    let (max_tokens, thinking, speed) =
        runner.agent_config.build_base_request_fields(&runner.model);
    let mut request = LlmRequest {
        model: runner.model.clone(),
        max_tokens,
        system: build_system_messages(runner, messages),
        // Filled by the round's single projection, in `stream_round`.
        messages: Vec::new(),
        // #171 part 3: an Arc bump, not a deep clone of ~70 frozen schemas.
        tools: archon_llm::provider::SharedTools::clone(&runner.tool_definitions),
        thinking,
        speed,
        effort: resolve_effort(runner).await,
        extra: runner.agent_config.auxiliary_runtime_extra(
            "subagent",
            "subagent",
            turn as u64,
            Some(turn),
            None,
        ),
        request_origin: Some("subagent".into()),
        reasoning_encrypted,
    };
    crate::agent::request_cache::apply_system_cache(
        &mut request,
        runner.provider.as_ref(),
        &crate::agent::request_cache::CacheSettings::from_context(&runner.agent_config.context),
    );
    // Runs against the empty message array on purpose: the conversation marker
    // itself is placed by the projection in `stream_round`, but on a provider
    // without message caching this is also what strips inherited markers off
    // the system blocks and the tool list, and that has to happen here too.
    crate::agent::request_cache::apply_conversation_cache(
        &mut request,
        runner.provider.as_ref(),
        runner.agent_config.context.prompt_cache_conversation,
        &crate::agent::request_cache::CacheSettings::from_context(&runner.agent_config.context),
    );
    request
}

fn build_system_messages(
    runner: &SubagentRunner,
    messages: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut system = Vec::new();
    let first_user_message = messages
        .first()
        .and_then(|message| message.get("content"))
        .and_then(first_text_content)
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
    system.extend(runner.request_system.iter().cloned());
    if let Some(ref reminder) = runner.critical_system_reminder {
        system.push(serde_json::json!({
            "type": "text",
            "text": format!("<system-reminder>{reminder}</system-reminder>"),
        }));
    }
    system
}

fn first_text_content(content: &serde_json::Value) -> Option<&str> {
    content.as_str().or_else(|| {
        content.as_array()?.iter().find_map(|block| {
            (block.get("type").and_then(|value| value.as_str()) == Some("text"))
                .then(|| block.get("text").and_then(|value| value.as_str()))
                .flatten()
        })
    })
}

/// Resolve the subagent's effort level, per-agent-def override winning over
/// the live `/effort` level.
///
/// #123: returns a concrete level rather than `None` for `High`. See
/// `Agent::turn_effort` for why that omission had to move down into the
/// provider adapters — the short version is that an absent effort field means
/// "high" on Anthropic but "no reasoning at all" on an OpenAI-compatible
/// backend, so it cannot be decided here.
async fn resolve_effort(runner: &SubagentRunner) -> Option<String> {
    if runner.effort.is_some() {
        return runner.effort.clone();
    }
    let level = *runner.agent_config.effort_level.lock().await;
    Some(level.to_string())
}

async fn maybe_compact_for_request_pressure(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    (pressure, envelope_bytes): (&mut PressureState, usize),
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
    if !(token_pressure || body_pressure) {
        return;
    }
    // Arithmetic before inference. `[prune]` may reclaim enough on its own,
    // and unlike the summariser it has no context window of its own to blow.
    if super::request_round_pressure::prune_history_mechanically(messages) > 0 {
        *request_body_bytes = crate::agent::autocompact::estimated_body_bytes(
            envelope_bytes,
            messages.estimated_tokens(),
        );
        let after = current_trigger_tokens(messages, *last_known_context_tokens);
        let still_over = runner
            .agent_config
            .context
            .rate_limit_pressure_tokens
            .is_some_and(|threshold| after >= threshold)
            || runner
                .agent_config
                .context
                .rate_limit_pressure_body_bytes
                .is_some_and(|threshold| *request_body_bytes as u64 >= threshold);
        if !still_over {
            // Under threshold again without spending a request. The watermark
            // is deliberately untouched: nothing was attempted, so nothing
            // should be held back later.
            return;
        }
    }
    if !super::request_round_pressure::proactive_rearmed(
        pressure.proactive_retry_watermark,
        trigger_tokens,
    ) || !auto_compact.should_attempt()
    {
        return;
    }
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
    let compacted = super::request_round_pressure::compact_proactively(
        runner,
        messages,
        auto_compact,
        last_known_context_tokens,
        telemetry,
        crate::agent::CompactAction::Full,
        "subagent request-pressure compaction failed; continuing turn",
    )
    .await;
    // Success re-arms unconditionally: the compacted history is the new
    // baseline and the pressure threshold alone decides when to act again.
    // Failure holds the guard at the size that failed, so the next attempt
    // waits for genuinely new material rather than re-summarising the same
    // bytes every turn.
    pressure.proactive_retry_watermark = if compacted {
        None
    } else {
        Some(current_trigger_tokens(messages, *last_known_context_tokens))
    };
    // The compacted history is projected once, when the request is opened.
    // Rebuilding the message array here as well was the second of the two
    // full-history passes #171 part 2 set out to remove.
    *request_body_bytes = crate::agent::autocompact::estimated_body_bytes(
        envelope_bytes,
        messages.estimated_tokens(),
    );
}

/// #103 trigger guard: the freshest of provider-reported usage and our own
/// estimate wins. Part 1 only made the estimate O(1); which number the guard
/// picks is unchanged.
fn current_trigger_tokens(messages: &MessageHistory, last_known_context_tokens: u64) -> u64 {
    last_known_context_tokens.max(messages.estimated_tokens())
}

fn pressure_reason(token_pressure: bool, body_pressure: bool) -> &'static str {
    match (token_pressure, body_pressure) {
        (true, true) => "request_pressure_tokens_and_bytes",
        (true, false) => "request_pressure_tokens",
        (false, true) => "request_pressure_bytes",
        (false, false) => unreachable!(),
    }
}

#[cfg(test)]
#[path = "request_round_trigger_tests.rs"]
mod trigger_tests;
