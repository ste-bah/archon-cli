//! Logical-call usage attribution and truthful usage normalization.

use archon_learning::llm_call_usage::{LlmCallUsageRecord, UsageAvailability};
use archon_llm::provider::LlmRequest;
use archon_llm::types::Usage;
use chrono::Utc;

#[derive(Clone)]
pub(crate) struct ObservedRequest {
    pub(crate) provider_id: String,
    pub(crate) model: String,
    pub(crate) origin: Option<String>,
    pub(crate) run_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) turn: Option<u64>,
    pub(crate) round: Option<u64>,
    pub(crate) role: Option<String>,
    pub(crate) denominator: Option<u64>,
    pub(crate) agent_type: Option<String>,
    pub(crate) agent_version: Option<String>,
}

impl ObservedRequest {
    pub(super) fn from_request(provider_id: &str, request: &LlmRequest) -> Self {
        let runtime = request.extra.get("archon_runtime");
        Self {
            provider_id: provider_id.into(),
            model: request.model.clone(),
            origin: runtime_field(runtime, "origin").or_else(|| request.request_origin.clone()),
            run_id: runtime_field(runtime, "run_id"),
            session_id: runtime_field(runtime, "session_id"),
            turn: runtime_u64(runtime, "turn"),
            round: runtime_u64(runtime, "round"),
            role: runtime_field(runtime, "role"),
            denominator: runtime_u64(runtime, "effective_denominator"),
            agent_type: runtime_field(runtime, "agent_type"),
            agent_version: runtime_field(runtime, "agent_version"),
        }
    }
}

pub(super) fn logical_call_usage(
    request_id: &str,
    request: &ObservedRequest,
    usage: Option<&Usage>,
    status: &str,
) -> LlmCallUsageRecord {
    LlmCallUsageRecord {
        request_id: request_id.into(),
        run_id: request.run_id.clone(),
        session_id: request.session_id.clone(),
        turn: request.turn,
        round: request.round,
        role: request.role.clone(),
        origin: request.origin.clone(),
        provider_id: request.provider_id.clone(),
        model_id: request.model.clone(),
        input_tokens: usage_availability(usage, |usage| {
            (usage.input_tokens, usage.input_tokens_available)
        }),
        output_tokens: usage_availability(usage, |usage| {
            (usage.output_tokens, usage.output_tokens_available)
        }),
        cache_creation_input_tokens: usage_availability(usage, |usage| {
            (
                usage.cache_creation_input_tokens,
                usage.cache_creation_input_tokens_available,
            )
        }),
        cache_read_input_tokens: usage_availability(usage, |usage| {
            (
                usage.cache_read_input_tokens,
                usage.cache_read_input_tokens_available,
            )
        }),
        context_input_tokens: usage.and_then(context_input_tokens),
        effective_denominator: request.denominator,
        terminal_status: status.into(),
        created_at: Utc::now().to_rfc3339(),
    }
}

fn runtime_field(runtime: Option<&serde_json::Value>, field: &str) -> Option<String> {
    runtime?
        .get(field)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn runtime_u64(runtime: Option<&serde_json::Value>, field: &str) -> Option<u64> {
    runtime?.get(field)?.as_u64()
}

fn usage_availability(
    usage: Option<&Usage>,
    field: impl Fn(&Usage) -> (u64, bool),
) -> UsageAvailability {
    match usage.map(field) {
        Some((value, true)) => UsageAvailability::Known(value),
        _ => UsageAvailability::Unavailable,
    }
}

/// Version 1 defines context input as base input plus both cache components.
/// It is unavailable unless every component is explicitly reported.
pub(super) fn context_input_tokens(usage: &Usage) -> Option<u64> {
    if usage.input_tokens_available
        && usage.cache_creation_input_tokens_available
        && usage.cache_read_input_tokens_available
    {
        usage
            .input_tokens
            .checked_add(usage.cache_creation_input_tokens)?
            .checked_add(usage.cache_read_input_tokens)
    } else {
        None
    }
}
