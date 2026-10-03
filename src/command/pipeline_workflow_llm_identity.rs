//! The request settings a configured pipeline client sends with every call
//! that can change what its provider answers, as one digest (Issue 259
//! review). A host that saves a verdict keys it by this digest beside the
//! provider and the resolved model, so a verdict is never reused after the
//! endpoint or the output ceiling changed.
//!
//! What is in it, read from the same settings the client is built from:
//!
//! - the configured provider name;
//! - the output ceiling the adapter sends as `max_tokens`
//!   ([`configured_output_ceiling`]);
//! - the Anthropic route under the client's endpoint policy, which serves
//!   every Anthropic call and the fallback when another provider cannot be
//!   built;
//! - the selected provider's own endpoint and model settings.
//!
//! Temperature, effort and thinking are not settings here: the adapter
//! sends none of them unless its caller passes them per call, and a caller's
//! per-call values are fixed by its binary, which the host keys separately.
//!
//! What is never in it: an API key, a token, a credentials file. Only the
//! digest leaves this module, so an endpoint is not written in clear either.

use archon_core::config::ArchonConfig;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::command::pipeline_support::configured_output_ceiling;
use crate::command::workflow_provider_route::{ProviderEndpointPolicy, resolve_anthropic_route};

/// The digest of `config`'s answer-changing request settings for a client
/// built under `policy`.
pub(crate) fn request_identity(config: &ArchonConfig, policy: ProviderEndpointPolicy) -> String {
    let llm = &config.llm;
    let selected = match llm.provider.as_str() {
        "openai" => json!({"base_url": llm.openai.base_url, "model": llm.openai.model}),
        "local" => json!({
            "base_url": llm.local.base_url,
            "model": llm.local.model,
            "reasoning": llm.local.reasoning,
        }),
        "bedrock" => json!({"region": llm.bedrock.region, "model_id": llm.bedrock.model_id}),
        "vertex" => json!({
            "project_id": llm.vertex.project_id,
            "region": llm.vertex.region,
            "model": llm.vertex.model,
        }),
        "openai-codex" => json!({
            "base_url": std::env::var("ARCHON_CODEX_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        }),
        _ => serde_json::Value::Null,
    };
    let anthropic = resolve_anthropic_route(config.api.base_url.as_deref(), policy);
    let material = json!({
        "provider": llm.provider,
        "max_tokens": configured_output_ceiling(config),
        "anthropic_endpoint": anthropic.endpoint,
        "selected": selected,
    });
    hex::encode(Sha256::digest(material.to_string().as_bytes()))
}

#[cfg(test)]
#[path = "pipeline_workflow_llm_identity_tests.rs"]
mod tests;
