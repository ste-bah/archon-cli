/// LlmProvider trait and supporting types for the multi-provider abstraction layer.
///
/// Design constraints:
/// - Trait must be object-safe (usable as `Box<dyn LlmProvider>` / `Arc<dyn LlmProvider>`)
/// - Uses `Receiver<StreamEvent>` instead of `Pin<Box<dyn Stream>>` to stay object-safe
///   and to match `AnthropicClient::stream_message`'s existing return type.
use std::collections::HashMap;

use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::anthropic::{AnthropicClient, MessageRequest};
use crate::streaming::StreamEvent;
use crate::types::Usage;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("server overloaded")]
    Overloaded,

    #[error("server error ({status}): {message}")]
    Server { status: u16, message: String },

    #[error("serialization error: {0}")]
    Serialize(String),

    #[error("feature not supported: {0}")]
    Unsupported(String),

    #[error("provider '{name}' not found — available: [{available}]")]
    ProviderNotFound { name: String, available: String },
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

/// Optional capabilities that vary across LLM providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFeature {
    /// Extended thinking / chain-of-thought tokens.
    Thinking,
    /// Function / tool calling.
    ToolUse,
    /// Prompt caching / cache_control breakpoints.
    PromptCaching,
    /// Multi-modal image input.
    Vision,
    /// System prompt injection.
    SystemPrompt,
    /// Server-sent event streaming.
    Streaming,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Provider-agnostic representation of an LLM inference request.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: Vec<serde_json::Value>,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub thinking: Option<serde_json::Value>,
    /// When fast mode is active, set to `Some("fast")`.
    pub speed: Option<String>,
    /// When effort is not High, set to the effort level string (e.g. `"low"`, `"medium"`).
    pub effort: Option<String>,
    /// Provider-specific escape hatch for parameters not in this struct.
    pub extra: serde_json::Value,
    /// Diagnostic marker for tracing: "main_session", "subagent", or None.
    pub request_origin: Option<String>,
}

impl Default for LlmRequest {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8192,
            system: Vec::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            thinking: None,
            speed: None,
            effort: None,
            extra: serde_json::Value::Null,
            request_origin: None,
        }
    }
}

/// Bidirectional conversion between `LlmRequest` and `MessageRequest`.
impl From<MessageRequest> for LlmRequest {
    fn from(mr: MessageRequest) -> Self {
        Self {
            model: mr.model,
            max_tokens: mr.max_tokens,
            system: mr.system,
            messages: mr.messages,
            tools: mr.tools,
            thinking: mr.thinking,
            speed: mr.speed,
            effort: mr.effort,
            extra: serde_json::Value::Null,
            request_origin: None,
        }
    }
}

impl From<LlmRequest> for MessageRequest {
    fn from(lr: LlmRequest) -> Self {
        Self {
            model: lr.model,
            max_tokens: lr.max_tokens,
            system: lr.system,
            messages: lr.messages,
            tools: lr.tools,
            thinking: lr.thinking,
            speed: lr.speed,
            effort: lr.effort,
            request_origin: lr.request_origin,
        }
    }
}

/// Provider-agnostic representation of a non-streaming LLM response.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<serde_json::Value>,
    pub usage: Usage,
    pub stop_reason: String,
}

/// Metadata about a model offered by a provider.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub context_window: u32,
}

// ---------------------------------------------------------------------------
// LlmProvider trait
// ---------------------------------------------------------------------------

/// Object-safe trait representing any LLM backend.
///
/// Implementors must be `Send + Sync` so they can be stored in `Arc<dyn LlmProvider>`
/// and shared across threads.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Short, stable identifier for this provider (e.g. `"anthropic"`, `"openai"`).
    fn name(&self) -> &str;

    /// List of models available through this provider.
    fn models(&self) -> Vec<ModelInfo>;

    /// Initiate a streaming request. Returns a channel receiver that yields
    /// `StreamEvent` values as they arrive from the backend.
    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError>;

    /// Perform a non-streaming (batch) completion. Collects the full response
    /// before returning.
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Report whether this provider supports an optional capability.
    fn supports_feature(&self, feature: ProviderFeature) -> bool;

    /// Downcast to the underlying `AnthropicClient` if this provider wraps one.
    ///
    /// Returns `None` for all non-Anthropic providers. Used by code paths that
    /// need Anthropic-specific headers (auth, identity).
    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        None
    }
}

// ---------------------------------------------------------------------------
// ProviderRegistry
// ---------------------------------------------------------------------------

/// Registry of all configured LLM providers, keyed by their `name()`.
pub struct ProviderRegistry {
    providers: HashMap<String, Box<dyn LlmProvider>>,
}

impl ProviderRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a provider. Replaces any existing provider with the same name.
    pub fn register(&mut self, provider: Box<dyn LlmProvider>) {
        let key = provider.name().to_string();
        self.providers.insert(key, provider);
    }

    /// Look up a provider by name. Returns `None` if not registered.
    pub fn get(&self, name: &str) -> Option<&dyn LlmProvider> {
        self.providers.get(name).map(|p| p.as_ref())
    }

    /// Return the provider for the given config name, or a descriptive `Err`
    /// listing all available providers.
    pub fn active(&self, config_provider: &str) -> Result<&dyn LlmProvider, LlmError> {
        self.get(config_provider).ok_or_else(|| {
            let available: Vec<&str> = self.providers.keys().map(|s| s.as_str()).collect();
            let mut sorted = available;
            sorted.sort_unstable();
            LlmError::ProviderNotFound {
                name: config_provider.to_string(),
                available: sorted.join(", "),
            }
        })
    }

    /// Iterate over all registered providers.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &dyn LlmProvider)> {
        self.providers.iter().map(|(k, v)| (k.as_str(), v.as_ref()))
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
