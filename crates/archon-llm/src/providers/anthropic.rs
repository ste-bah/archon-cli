/// `AnthropicProvider` — wraps `AnthropicClient` and implements `LlmProvider`.
///
/// Converts between the provider-agnostic `LlmRequest` / `LlmResponse` types
/// and the Anthropic-specific `MessageRequest` / `StreamEvent` types without
/// modifying the underlying `AnthropicClient`.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::anthropic::{AnthropicClient, ApiError};
use crate::completion_accumulator::collect_completion_response;
use crate::context_window::classify_context_window_error;
use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature, classify_data_flow_endpoint,
};
use crate::streaming::StreamEvent;

// ---------------------------------------------------------------------------
// Error conversion
// ---------------------------------------------------------------------------

impl From<ApiError> for LlmError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::HttpError(msg) => {
                classify_context_window_error(None, None, None, &msg, Some("anthropic"), None)
                    .unwrap_or(LlmError::Http(msg))
            }
            ApiError::AuthError(msg) => LlmError::Auth(msg),
            ApiError::RateLimited { retry_after_secs } => {
                LlmError::RateLimited { retry_after_secs }
            }
            ApiError::Overloaded => LlmError::Overloaded,
            ApiError::ServerError { status, message } => classify_context_window_error(
                Some(status),
                None,
                None,
                &message,
                Some("anthropic"),
                None,
            )
            .unwrap_or(LlmError::Server { status, message }),
            ApiError::SerializeError(msg) => LlmError::Serialize(msg),
        }
    }
}

// ---------------------------------------------------------------------------
// AnthropicProvider
// ---------------------------------------------------------------------------

/// Anthropic tier alias map — provider-owned model identifiers indexed by
/// agent class.
///
/// Defaults match `archon_core::config::AnthropicModelsConfig::default()`.
/// The binary should populate this from the operator's `[models.anthropic]`
/// config and pass it to `AnthropicProvider::with_alias_map(..)` so config
/// overrides flow through to provider resolution.
#[derive(Debug, Clone)]
pub struct AnthropicAliasMap {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
}

impl Default for AnthropicAliasMap {
    /// These literals must equal `[models.anthropic]` in the shipped
    /// `config.toml`, and `anthropic_alias_map_tracks_template.rs` fails the
    /// build if they drift.
    ///
    /// They had already drifted once: `opus` said `claude-opus-4-8` while the
    /// template said `claude-opus-5`. `archon-core` fixed the same class of bug
    /// on its side by reading the template at runtime, and left a comment
    /// naming `claude-opus-4-8` as the stale value — but `archon-core` sits
    /// *above* `archon-llm` and cannot lend it that reader, so this copy kept
    /// the literal and went unnoticed. The Codex map next door had a guard test;
    /// this one did not, which is the whole reason it rotted.
    ///
    /// The staleness is not cosmetic: an alias resolves to a model id, and the
    /// id decides the prompt-cache minimum. `claude-opus-4-8` asks for a
    /// 1,024-token prefix where `claude-opus-5` needs only 512.
    fn default() -> Self {
        Self {
            opus: "claude-opus-5".into(),
            sonnet: "claude-sonnet-5".into(),
            haiku: "claude-haiku-4-5".into(),
        }
    }
}

/// Display names for ids this build has heard of.
///
/// Presentation only — nothing resolves through it, and an id missing from it
/// still enumerates, under its own name. That matters because the list of models
/// worth offering changes faster than releases do, and the last version of this
/// list to carry behaviour with it went stale pointing at `claude-opus-4-8`.
const ANTHROPIC_KNOWN_MODELS: &[(&str, &str)] = &[
    ("claude-opus-5", "Claude Opus 5"),
    ("claude-sonnet-5", "Claude Sonnet 5"),
    ("claude-haiku-4-5", "Claude Haiku 4.5"),
    ("claude-opus-4-8", "Claude Opus 4.8"),
    ("claude-opus-4-7", "Claude Opus 4.7"),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
];

/// `context_window: 0` throughout, deliberately.
///
/// `context_window::for_model` consults the user and bundled catalogs first and
/// treats zero as "ask them"; a number invented here would have to be right for
/// every model listed, and would silently win over a catalog that was.
fn anthropic_model_info(id: &str) -> ModelInfo {
    let display_name = ANTHROPIC_KNOWN_MODELS
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| id.to_string());
    ModelInfo {
        id: id.to_string(),
        display_name,
        context_window: 0,
    }
}

fn push_unique_model(models: &mut Vec<ModelInfo>, id: &str) {
    if id.is_empty() || models.iter().any(|model| model.id == id) {
        return;
    }
    models.push(anthropic_model_info(id));
}

/// An `LlmProvider` backed by `AnthropicClient`.
///
/// The inner client remains accessible via `client()` for code paths that
/// need Anthropic-specific accessors (auth headers, identity headers).
pub struct AnthropicProvider {
    client: AnthropicClient,
    aliases: AnthropicAliasMap,
}

impl AnthropicProvider {
    /// Wrap an existing `AnthropicClient` using compile-time-default aliases.
    ///
    /// Use `with_alias_map(..)` to supply an operator-overridden alias map
    /// from `ArchonConfig::models.anthropic`.
    pub fn new(client: AnthropicClient) -> Self {
        Self {
            client,
            aliases: AnthropicAliasMap::default(),
        }
    }

    /// Builder: attach an alias map sourced from operator config.
    pub fn with_alias_map(mut self, aliases: AnthropicAliasMap) -> Self {
        self.aliases = aliases;
        self
    }

    /// Access the underlying `AnthropicClient` directly.
    ///
    /// Used to reach `auth()` and `identity()` for header injection in code
    /// paths that must remain Anthropic-aware.
    pub fn client(&self) -> &AnthropicClient {
        &self.client
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    /// Configured models first, then the known catalog.
    ///
    /// The head of this list is load-bearing, not merely informational:
    /// `resolve_request_model` falls back to `models().first()` when a request
    /// carries no model, so whatever leads here *is* the effective default. A
    /// hardcoded list therefore does not go stale quietly, it overrides config —
    /// this one led with `claude-opus-4-8` while `[models.anthropic]` said
    /// `claude-opus-5`, which is the same bug the Codex provider next door
    /// already fixed for `gpt-5.5`.
    ///
    /// It is not only a wrong name. The resolved id selects the prompt-cache
    /// minimum, and those do not follow the version number: `claude-opus-4-8`
    /// asks for a 1,024-token prefix where `claude-opus-5` caches from 512.
    ///
    /// The known catalog still follows so enumeration never loses an id.
    fn models(&self) -> Vec<ModelInfo> {
        let mut models: Vec<ModelInfo> = Vec::new();
        for id in [
            &self.aliases.opus,
            &self.aliases.sonnet,
            &self.aliases.haiku,
        ] {
            push_unique_model(&mut models, id);
        }
        for (id, _) in ANTHROPIC_KNOWN_MODELS {
            push_unique_model(&mut models, id);
        }
        models
    }

    fn resolve_alias(&self, alias: &str) -> Option<String> {
        match alias.trim().to_lowercase().as_str() {
            "opus" => Some(self.aliases.opus.clone()),
            "sonnet" => Some(self.aliases.sonnet.clone()),
            "haiku" => Some(self.aliases.haiku.clone()),
            _ => None,
        }
    }

    async fn stream(&self, mut request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.resolve_request_model(&mut request);
        request.messages = crate::message_invariants::sanitize_anthropic_shape(request.messages);
        let msg_request = request.into();
        self.client
            .stream_message(msg_request)
            .await
            .map_err(LlmError::from)
    }

    /// Collect a full non-streaming response by consuming all stream events.
    ///
    /// Drives the same `stream_message` path underneath and collects
    /// stream content blocks and usage tokens into an `LlmResponse`.
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        collect_completion_response(self.stream(request).await?).await
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        match feature {
            ProviderFeature::PromptCaching => {
                crate::anthropic_url::is_official_messages_url(self.client.api_url())
            }
            ProviderFeature::Thinking
            | ProviderFeature::ToolUse
            | ProviderFeature::Vision
            | ProviderFeature::SystemPrompt
            | ProviderFeature::Streaming => true,
        }
    }

    /// The official Messages endpoint is known to preserve `cache_control`.
    ///
    /// Anything else is left at `None` here and must be declared through
    /// `[llm] cache_strategy` instead. That is deliberately conservative: a
    /// gateway that mangles the directive would 400 on every request, which is
    /// worse than paying more. It is also the reason gateway deployments were
    /// silently paying full price for every token, so the override exists to be
    /// used.
    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        if crate::anthropic_url::is_official_messages_url(self.client.api_url()) {
            crate::cache_strategy::anthropic_for_model(model, self.cache_platform())
        } else {
            crate::cache_strategy::CacheStrategy::None
        }
    }

    /// The official endpoint is the first-party API — by API key or by
    /// subscription OAuth, which share both the URL and the limits.
    ///
    /// Anything else is a gateway, and a gateway's URL says nothing about what
    /// is behind it. `Unknown` there is what stops an operator who declares
    /// `cache_strategy = "anthropic"` in front of Bedrock from silently emitting
    /// checkpoints under Bedrock's floor.
    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        if crate::anthropic_url::is_official_messages_url(self.client.api_url()) {
            crate::cache_models::CachePlatform::AnthropicApi
        } else {
            crate::cache_models::CachePlatform::Unknown
        }
    }

    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        Some(&self.client)
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        classify_data_flow_endpoint(self.client.api_url())
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        match &self.client.identity().mode {
            crate::identity::IdentityMode::Spoof { .. } => {
                crate::compaction_policy::ProviderFamily::AnthropicOAuth
            }
            crate::identity::IdentityMode::Clean | crate::identity::IdentityMode::Custom { .. } => {
                crate::compaction_policy::ProviderFamily::AnthropicApi
            }
        }
    }
}
