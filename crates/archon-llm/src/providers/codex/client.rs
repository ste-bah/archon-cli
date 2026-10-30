use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use tokio::sync::{Mutex, OnceCell};
use uuid::Uuid;

use crate::auth::{AuthError, CodexCredentials};
use crate::completion_accumulator::collect_completion_response;
use crate::oauth_codex::CodexOAuthClient;
use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature, classify_data_flow_endpoint,
};
use crate::providers::codex::sse::forward_codex_sse;
use crate::providers::codex::tls_preflight::run_codex_tls_preflight_url;
use crate::streaming::StreamEvent;
use crate::tokens_codex::{
    ensure_codex_token_valid, read_codex_credentials_locked, write_codex_credentials_atomic,
};

use super::spoof::SpoofConfig;
use super::translator::{
    StreamAccumulator, join_system_prompt, messages_to_responses_input, tools_to_responses_tools,
};
use super::types::{ReasoningConfig, ResponsesRequest, TextConfig};

const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_PREFLIGHT_URL: &str = "https://auth.openai.com/";
const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 1_000;
const RESERVED_HEADERS: &[&str] = &[
    "authorization",
    "chatgpt-account-id",
    "content-type",
    "accept",
    "session_id",
    "x-client-request-id",
    "user-agent",
    "openai-beta",
];

/// Codex tier alias map — provider-owned model identifiers indexed by tier
/// keyword.
///
/// The binary should populate from operator config and pass via
/// `CodexProvider::with_alias_map(..)`; `default()` is the fallback for callers
/// that cannot (e.g. `build_llm_provider`, which takes the flat `LlmConfig` and
/// so has no `[models.openai-codex]` slice to read).
///
/// Tier mapping, identical to `OpenAiCodexModelsConfig::to_alias_map()`:
/// - `opus`   → template `default` (frontier flagship)
/// - `sonnet` → template `default` (same model for now)
/// - `haiku`  → template `mini` (efficient/subagent variant)
/// - `codex`  → template `codex` (codex-specific software engineering)
#[derive(Debug, Clone)]
pub struct CodexAliasMap {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
    pub codex: String,
}

/// `[models.openai-codex]` from the shipped `config.toml`.
///
/// `archon-llm` sits below `archon-core` in the workspace dependency order, so it
/// cannot call `OpenAiCodexModelsConfig::default()`. It embeds the same
/// workspace-root file instead: one source of truth, reached independently,
/// without inverting the dependency graph. `codex_defaults_track_the_template`
/// pins the two readings together.
fn template_codex_model(key: &str) -> String {
    static TEMPLATE: LazyLock<toml::Value> = LazyLock::new(|| {
        include_str!("../../../../../config.toml")
            .parse::<toml::Value>()
            .expect("shipped config.toml must be valid TOML")
    });
    TEMPLATE
        .get("models")
        .and_then(|models| models.get("openai-codex"))
        .and_then(|slice| slice.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("shipped config.toml is missing models.openai-codex.{key}"))
        .to_string()
}

impl Default for CodexAliasMap {
    fn default() -> Self {
        Self {
            opus: template_codex_model("default"),
            sonnet: template_codex_model("default"),
            haiku: template_codex_model("mini"),
            codex: template_codex_model("codex"),
        }
    }
}

/// Codex model ids whose display name and context window are known here.
///
/// Not a default-selection table — `models()` leads with the configured alias
/// map and appends these so enumeration stays complete.
///
/// Nor is it the primary context-window source: `context_window::for_model`
/// checks the user and bundled catalogs (`resources/context.toml`) first and
/// only falls through to provider metadata. Adding an id here is not how a model
/// gets an accurate window — the catalog is.
const CODEX_KNOWN_MODELS: &[(&str, &str, u32)] = &[
    ("gpt-5.5", "GPT-5.5", 1_050_000),
    ("gpt-5.4", "GPT-5.4", 1_050_000),
    ("gpt-5.4-mini", "GPT-5.4 Mini", 400_000),
    ("gpt-5.3-codex", "GPT-5.3 Codex", 400_000),
];

/// Context window assumed for a configured id absent from `CODEX_KNOWN_MODELS`.
///
/// Reached only when neither context catalog knows the id either, since those
/// are consulted first — so this is a last resort, not the usual path. It is
/// deliberately the smaller of the two known windows: under-promising truncates
/// a request, where over-promising has the API reject it outright.
/// `[api].context_window_override` raises it.
const CODEX_UNKNOWN_CONTEXT_WINDOW: u32 = 400_000;

fn codex_model_info(id: &str) -> ModelInfo {
    match CODEX_KNOWN_MODELS.iter().find(|(known, _, _)| *known == id) {
        Some((_, display_name, context_window)) => ModelInfo {
            id: id.to_string(),
            display_name: (*display_name).to_string(),
            context_window: *context_window,
        },
        None => ModelInfo {
            id: id.to_string(),
            display_name: id.to_string(),
            context_window: CODEX_UNKNOWN_CONTEXT_WINDOW,
        },
    }
}

/// Append `id` unless it is empty or already listed, preserving first-seen order
/// — which is what makes `models().first()` the configured model.
/// Whether this model takes explicit `prompt_cache_breakpoint` markers.
///
/// GPT-5.6 is the cutoff; everything before it caches automatically on a stable
/// prefix and accepts no annotation. Parsed rather than listed because the ids
/// carry suffixes (`gpt-5.6-sol`, `gpt-5.3-codex`) and a fixed list would miss
/// the next one silently.
fn supports_responses_breakpoints(model: &str) -> bool {
    let lowered = model.trim().to_ascii_lowercase();
    let Some(rest) = lowered.strip_prefix("gpt-") else {
        return false;
    };
    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = version.split('.');
    let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    (major, minor) >= (5, 6)
}

/// A stable routing key for the prompt cache.
///
/// This used to be `Uuid::new_v4()`, minted inside `build_request_body` and so
/// fresh on every request. That is the one value the field must not take.
/// `prompt_cache_key` is how the service groups requests that share a prefix:
/// requests carrying the same key are routed to the same cache, and OpenAI
/// documents that a high-cardinality key lowers the hit rate. A key that never
/// repeats therefore guarantees a miss on every turn of every session, however
/// stable the prefix in front of it.
///
/// It hashes exactly the cached prefix — model, instructions, tool schemas — and
/// deliberately **not** the conversation input. Including the messages would
/// rebuild the original bug by hand, since they change every turn. Two turns of
/// one session share a key; a session with different tools or a different system
/// prompt gets its own, which is right, because a different prefix cannot hit
/// the same cache anyway.
///
/// Hashing the prefix rather than using a session id is the deliberate choice:
/// two archon sessions with the same tools and system prompt genuinely can share
/// a cached prefix, and a per-session key would stop them. The documented limit
/// on that is throughput — OpenAI routes on the key combined with a hash of the
/// leading tokens, and a single prefix-plus-key pair saturates around 15
/// requests per minute before excess traffic is spread over more machines, each
/// spread costing a one-time miss. One interactive CLI is nowhere near that; a
/// deployment fanning out many concurrent sessions would want the key salted per
/// session, and this is the function to do it in.
fn prompt_cache_key(
    model: &str,
    instructions: Option<&str>,
    tools: Option<&[super::types::ResponseTool]>,
) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update([0]);
    hasher.update(instructions.unwrap_or_default().as_bytes());
    hasher.update([0]);
    if let Some(tools) = tools
        && let Ok(encoded) = serde_json::to_vec(tools)
    {
        hasher.update(&encoded);
    }

    // Half the digest is ample for a routing key and keeps the header-adjacent
    // body field short.
    let digest = hasher.finalize();
    digest[..16].iter().fold(String::new(), |mut key, byte| {
        use std::fmt::Write;
        let _ = write!(key, "{byte:02x}");
        key
    })
}

fn push_unique_model(models: &mut Vec<ModelInfo>, id: &str) {
    if id.is_empty() || models.iter().any(|model| model.id == id) {
        return;
    }
    models.push(codex_model_info(id));
}

pub struct CodexProvider {
    http: reqwest::Client,
    credentials_path: PathBuf,
    spoof: SpoofConfig,
    base_url: String,
    oauth_client: Arc<CodexOAuthClient>,
    tls_preflight_cache: OnceCell<()>,
    refresh_lock: Mutex<()>,
    preflight_url: String,
    retry_base_delay: Duration,
    aliases: CodexAliasMap,
}

#[path = "client_request.rs"]
mod request_impl;

#[async_trait]
impl LlmProvider for CodexProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    /// The Codex subscription and an OpenAI API key reach the same service; only
    /// the credential differs, and a credential does not move a threshold.
    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        crate::cache_models::CachePlatform::OpenAiApi
    }

    /// GPT-5.5 and earlier cache automatically on a stable prefix, with nothing
    /// to annotate. That is [`CacheStrategy::Automatic`] rather than
    /// [`CacheStrategy::None`], and the distinction matters in both directions:
    /// `Automatic` reports as caching, so these models are not shown as
    /// uncached, but it emits no markers, because a marker here would be an
    /// unsupported field rather than a harmless hint.
    ///
    /// From GPT-5.6 the Responses API accepts explicit breakpoints, and the
    /// strategy carries their limits — but archon does not emit them here, for a
    /// documented reason rather than an unfinished one.
    ///
    /// A breakpoint has to sit on a content block, and OpenAI states plainly
    /// that top-level `instructions` cannot carry one; reusable instructions
    /// have to be moved into an `input_text` block inside a developer message to
    /// be markable. Archon's stable content is exactly that top-level
    /// `instructions` field, and this provider impersonates the Codex CLI, whose
    /// captured traffic puts it there. Restructuring the request to mark it
    /// would change the shape being impersonated.
    ///
    /// Little is lost. The default `implicit` mode already places a breakpoint
    /// at the latest message, so the prefix it caches is instructions, tools and
    /// history — the same span an explicit breakpoint after the instructions
    /// would cover, plus more. What that mode genuinely needs is a stable
    /// `prompt_cache_key`, which it was not getting; see `prompt_cache_key`.
    ///
    /// Emitting `prompt_cache_options` would also be actively unsafe on the
    /// shared path: models before 5.6 reject both it and `prompt_cache_breakpoint`
    /// outright.
    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        if !supports_responses_breakpoints(model) {
            return crate::cache_strategy::CacheStrategy::Automatic;
        }
        let params = crate::cache_models::ModelCacheTable::default()
            .lookup_on(model, crate::cache_models::CachePlatform::OpenAiApi);
        crate::cache_strategy::CacheStrategy::ResponsesBreakpoints {
            max: params.max_checkpoints,
            min_tokens: params.min_tokens,
        }
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        crate::compaction_policy::ProviderFamily::CodexOAuth
    }

    fn models(&self) -> Vec<ModelInfo> {
        // Configured models first, then the known catalog.
        //
        // The head of this list is load-bearing, not merely informational:
        // `resolve_request_model` falls back to `models().first()` when a request
        // carries no model, so whatever leads here *is* the effective default. A
        // hardcoded list therefore does not just go stale, it overrides config —
        // this one led with `gpt-5.5` while `[models.openai-codex]` said
        // `gpt-5.6-sol`.
        //
        // Enumerating every configured id also matters, though less acutely:
        // `context_window::for_model` consults the user and bundled catalogs
        // first and only then looks the id up here, so a missing id costs a
        // window only for a model no catalog knows either.
        //
        // The known catalog still follows so enumeration never loses an id.
        let mut models: Vec<ModelInfo> = Vec::new();
        for id in [
            &self.aliases.opus,
            &self.aliases.sonnet,
            &self.aliases.codex,
            &self.aliases.haiku,
        ] {
            push_unique_model(&mut models, id);
        }
        for (id, _, _) in CODEX_KNOWN_MODELS {
            push_unique_model(&mut models, id);
        }
        models
    }

    fn resolve_alias(&self, alias: &str) -> Option<String> {
        // Map Anthropic-style tier aliases into Codex's namespace using the
        // operator-configurable alias map. Values come from
        // `[models.openai-codex]` in config.toml at provider construction time.
        match alias.trim().to_lowercase().as_str() {
            "opus" => Some(self.aliases.opus.clone()),
            "sonnet" | "default" => Some(self.aliases.sonnet.clone()),
            "haiku" | "mini" => Some(self.aliases.haiku.clone()),
            "codex" => Some(self.aliases.codex.clone()),
            _ => None,
        }
    }

    async fn stream(
        &self,
        mut request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.resolve_request_model(&mut request);
        self.ensure_preflight().await?;
        let body = self.build_request_body(&request)?;
        let wire_body_bytes = serialized_len(&body);
        let origin = request.request_origin.as_deref().unwrap_or("unknown");
        tracing::info!(
            target: "archon::context",
            provider = "openai-codex",
            request_origin = origin,
            request_model = %body.model,
            request_wire_body_bytes = wire_body_bytes,
            request_approx_wire_tokens = approx_tokens_from_bytes(wire_body_bytes),
            request_instructions_bytes = body
                .instructions
                .as_ref()
                .map(|instructions| instructions.len())
                .unwrap_or(0),
            request_input_bytes = serialized_len(&body.input),
            request_tools_bytes = body.tools.as_ref().map(serialized_len).unwrap_or(0),
            request_input_items = body.input.len(),
            request_tool_count = body.tools.as_ref().map(|tools| tools.len()).unwrap_or(0),
            "codex wire request size preflight"
        );
        let response = match self.send_with_retry(&body).await {
            Ok(response) => response,
            Err(err) => {
                if err.is_context_window_exceeded() {
                    tracing::warn!(
                        target: "archon::context",
                        provider = "openai-codex",
                        request_origin = origin,
                        request_model = %body.model,
                        request_wire_body_bytes = wire_body_bytes,
                        request_approx_wire_tokens = approx_tokens_from_bytes(wire_body_bytes),
                        error = %err,
                        "codex context window error with request size diagnostics"
                    );
                }
                return Err(err);
            }
        };
        let byte_stream = response.bytes_stream();

        let (raw_tx, mut raw_rx) = tokio::sync::mpsc::channel(256);
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        tokio::spawn(forward_codex_sse(byte_stream, raw_tx));
        tokio::spawn(async move {
            let mut accumulator = StreamAccumulator::default();
            loop {
                let item = tokio::select! {
                    _ = tx.closed() => return,
                    item = raw_rx.recv() => item,
                };
                let Some(item) = item else {
                    break;
                };
                match item {
                    Ok(event) => {
                        for translated in accumulator.process(event) {
                            match translated {
                                Ok(event) => {
                                    if tx.send(event).await.is_err() {
                                        return;
                                    }
                                }
                                Err(e) => {
                                    let _ = tx
                                        .send(StreamEvent::Error {
                                            error_type: "translator_error".into(),
                                            message: e.to_string().into(),
                                        })
                                        .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(crate::provider_fault::error_event(e)).await;
                        return;
                    }
                }
            }
        });
        Ok(rx)
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        collect_completion_response(self.stream(request).await?).await
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(
            feature,
            ProviderFeature::Streaming
                | ProviderFeature::ToolUse
                | ProviderFeature::Thinking
                | ProviderFeature::Vision
                | ProviderFeature::SystemPrompt
        )
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        classify_data_flow_endpoint(&self.base_url)
    }
}

#[path = "client_protocol.rs"]
mod protocol;
use protocol::{
    approx_tokens_from_bytes, auth_to_llm_error, is_quota_exceeded, is_retryable, map_codex_error,
    serialized_len,
};
pub use protocol::{
    build_codex_headers, build_reasoning_config, clamp_reasoning_effort, validate_spoof_headers,
};

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod recovery_tests;
