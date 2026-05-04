use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::anthropic::AnthropicProvider;
use archon_llm::providers::build_llm_provider;
use archon_llm::providers::codex::client::CodexProvider;
use archon_llm::providers::codex::spoof::resolve;
use archon_llm::streaming::StreamEvent;

use crate::cli_args::ChatArgs;

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
        stream_response(provider.as_ref(), request).await?;
    }
    Ok(())
}

async fn build_provider(
    args: &ChatArgs,
    config: &archon_core::config::ArchonConfig,
) -> Result<Arc<dyn LlmProvider>> {
    match args.provider.as_str() {
        "anthropic" => Ok(Arc::new(AnthropicProvider::new(
            build_anthropic_client(config).await?,
        ))),
        "openai-codex" => {
            let codex_cfg =
                crate::command::auth::codex_config_from_core(&config.providers.openai_codex);
            let resolution = resolve(&codex_cfg, &reqwest::Client::new())
                .await
                .context("failed to resolve Codex spoof identity")?;
            let provider = match std::env::var("ARCHON_CODEX_BASE_URL").ok() {
                Some(base_url) if !base_url.trim().is_empty() => CodexProvider::new_with_base_url(
                    archon_llm::tokens::credentials_path(),
                    resolution.config,
                    reqwest::Client::new(),
                    base_url,
                ),
                _ => CodexProvider::new(
                    archon_llm::tokens::credentials_path(),
                    resolution.config,
                    reqwest::Client::new(),
                ),
            }
            .context("failed to construct Codex provider")?;
            Ok(Arc::new(provider))
        }
        other => {
            let flat = archon_llm::LlmConfig {
                provider: other.into(),
                model: args.model.clone(),
                base_url: None,
                api_key_env: None,
                retry: None,
            };
            build_llm_provider(&flat, Arc::new(reqwest::Client::new()))
                .map_err(|e| anyhow::anyhow!("unknown or unavailable provider `{other}`: {e}"))
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

async fn stream_response(provider: &dyn LlmProvider, request: LlmRequest) -> Result<()> {
    let mut rx = provider.stream(request).await?;
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::TextDelta { text, .. } => {
                print!("{text}");
                io::stdout().flush()?;
            }
            StreamEvent::Error { message, .. } => return Err(anyhow::anyhow!(message)),
            StreamEvent::MessageStop => break,
            _ => {}
        }
    }
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
