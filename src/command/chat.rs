use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use archon_llm::agentic::{
    AgenticLlmProvider, AgenticTurnEvent, LlmProviderAgenticAdapter, ProviderCapabilitySet,
    TurnEventSink,
};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::anthropic::AnthropicProvider;
use archon_llm::providers::build_llm_provider;

use crate::cli_args::ChatArgs;
use crate::runtime::provider_observer::{
    observe_llm_provider_with_profile, runtime_mode_for_provider_name,
};

pub async fn handle_chat(args: ChatArgs, config: &archon_core::config::ArchonConfig) -> Result<()> {
    let provider = build_provider(&args, config).await?;
    let request = LlmRequest {
        model: args
            .model
            .clone()
            .unwrap_or_else(|| default_model(&args.provider, config)),
        max_tokens: args.max_tokens,
        system: vec![serde_json::json!({
            "type": "text",
            "text": "You are Archon, a concise and helpful AI assistant."
        })],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": args.prompt}]
        })],
        request_origin: Some("cli_chat".into()),
        ..LlmRequest::default()
    };

    if args.no_stream {
        let response = provider.complete(request).await?;
        println!("{}", response_text(&response.content));
    } else {
        stream_response(provider, request).await?;
    }
    Ok(())
}

async fn build_provider(
    args: &ChatArgs,
    config: &archon_core::config::ArchonConfig,
) -> Result<Arc<dyn LlmProvider>> {
    match args.provider.as_str() {
        "anthropic" => {
            let provider: Arc<dyn LlmProvider> = Arc::new(AnthropicProvider::new(
                build_anthropic_client(config).await?,
            ));
            let profile_id =
                crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                    provider.name(),
                );
            Ok(observe_llm_provider_with_profile(
                provider, "direct", profile_id,
            ))
        }
        "openai-codex" => {
            let (provider, runtime_mode) =
                crate::runtime::codex_provider::build_codex_provider(config, "cli_chat").await?;
            let profile_id =
                crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                    provider.name(),
                );
            Ok(observe_llm_provider_with_profile(
                provider,
                runtime_mode,
                profile_id,
            ))
        }
        other => {
            let flat = archon_llm::LlmConfig {
                provider: other.into(),
                model: args.model.clone(),
                base_url: None,
                api_key_env: None,
                retry: None,
            };
            let provider = build_llm_provider(&flat, Arc::new(reqwest::Client::new()))
                .map_err(|e| anyhow::anyhow!("unknown or unavailable provider `{other}`: {e}"))?;
            let runtime_mode = runtime_mode_for_provider_name(provider.name());
            let profile_id =
                crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                    provider.name(),
                );
            Ok(observe_llm_provider_with_profile(
                provider,
                runtime_mode,
                profile_id,
            ))
        }
    }
}

async fn build_anthropic_client(
    config: &archon_core::config::ArchonConfig,
) -> Result<archon_llm::anthropic::AnthropicClient> {
    let auth = archon_llm::auth::resolve_auth_with_keys(
        std::env::var("ANTHROPIC_API_KEY").ok().as_deref(),
        std::env::var("ARCHON_API_KEY").ok().as_deref(),
        std::env::var("ARCHON_OAUTH_TOKEN").ok().as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    )
    .context("Anthropic authentication unavailable")?;
    let identity_mode =
        archon_llm::identity::resolve_identity_mode(&auth, false, &config.identity.as_view());
    let account_uuid = if matches!(
        identity_mode,
        archon_llm::identity::IdentityMode::Spoof { .. }
    ) {
        crate::command::utils::fetch_account_uuid(&auth).await
    } else {
        String::new()
    };
    let identity = archon_llm::identity::IdentityProvider::new(
        identity_mode,
        uuid::Uuid::new_v4().to_string(),
        archon_llm::identity::get_or_create_device_id(),
        account_uuid,
    );
    let api_url = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| config.api.base_url.clone());
    Ok(archon_llm::anthropic::AnthropicClient::new(
        auth, identity, api_url,
    ))
}

async fn stream_response(provider: Arc<dyn LlmProvider>, request: LlmRequest) -> Result<()> {
    let provider_id = provider.name().to_string();
    let model_id = request.model.clone();
    let capabilities = ProviderCapabilitySet::from_llm_provider(provider.as_ref());
    let adapter = LlmProviderAgenticAdapter::new(provider, provider_id, model_id, capabilities);
    let (sink, mut events) = TurnEventSink::channel(256);
    let task = archon_observability::spawn_named("chat-stream", async move {
        adapter.stream_turn(request.into(), sink).await
    });

    while let Some(event) = events.recv().await {
        match event {
            AgenticTurnEvent::TextDelta { text } => {
                print!("{text}");
                io::stdout().flush()?;
            }
            AgenticTurnEvent::ProviderError { .. } => {}
            AgenticTurnEvent::TurnCompleted { .. } => {}
            _ => {}
        }
    }
    task.await.context("agentic chat stream task failed")??;
    Ok(())
}

fn response_text(content: &[serde_json::Value]) -> String {
    content
        .iter()
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

fn default_model(provider: &str, config: &archon_core::config::ArchonConfig) -> String {
    match provider {
        "anthropic" => config.api.default_model.clone(),
        "openai-codex" => "gpt-5.4".into(),
        other => archon_llm::providers::get_native(other)
            .or_else(|| archon_llm::providers::get_compat(other))
            .map(|d| d.default_model.clone())
            .unwrap_or_else(|| "claude-sonnet-4-6".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_text_joins_text_blocks_in_order() {
        let content = vec![
            serde_json::json!({"type": "text", "text": "hello"}),
            serde_json::json!({"type": "tool_use", "id": "ignored"}),
            serde_json::json!({"type": "text", "text": " world"}),
        ];

        assert_eq!(response_text(&content), "hello world");
    }

    #[test]
    fn default_model_uses_codex_default_for_codex_provider() {
        let config = archon_core::config::ArchonConfig::default();
        assert_eq!(default_model("openai-codex", &config), "gpt-5.4");
    }

    #[test]
    fn default_model_uses_configured_anthropic_model() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.api.default_model = "claude-test-model".into();

        assert_eq!(default_model("anthropic", &config), "claude-test-model");
    }
}
