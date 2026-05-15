use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tokio::sync::{Mutex, OnceCell};
use uuid::Uuid;

use crate::auth::{AuthError, CodexCredentials};
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
use crate::types::Usage;

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
/// Defaults match `archon_core::config::OpenAiCodexModelsConfig::default()`.
/// The binary should populate from operator config and pass via
/// `CodexProvider::with_alias_map(..)`.
///
/// Tier mapping (per OpenAI's Codex models reference):
/// - `opus`   → `gpt-5.4` (frontier flagship for tools/agentic)
/// - `sonnet` → `gpt-5.5` (newest general-purpose default)
/// - `haiku`  → `gpt-5.4-mini` (efficient/subagent variant)
/// - `codex`  → `gpt-5.3-codex` (codex-specific software engineering)
#[derive(Debug, Clone)]
pub struct CodexAliasMap {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
    pub codex: String,
}

impl Default for CodexAliasMap {
    fn default() -> Self {
        Self {
            opus: "gpt-5.4".into(),
            sonnet: "gpt-5.5".into(),
            haiku: "gpt-5.4-mini".into(),
            codex: "gpt-5.3-codex".into(),
        }
    }
}

pub struct CodexProvider {
    http: reqwest::Client,
    credentials_path: PathBuf,
    spoof: SpoofConfig,
    base_url: String,
    oauth_client: Arc<CodexOAuthClient>,
    tls_preflight_cache: OnceCell<Result<(), String>>,
    refresh_lock: Mutex<()>,
    preflight_url: String,
    retry_base_delay: Duration,
    aliases: CodexAliasMap,
}

impl CodexProvider {
    pub fn new(
        credentials_path: PathBuf,
        spoof: SpoofConfig,
        http: reqwest::Client,
    ) -> Result<Self, LlmError> {
        Self::new_inner(
            credentials_path,
            spoof,
            http,
            DEFAULT_BASE_URL.into(),
            DEFAULT_PREFLIGHT_URL.into(),
            Duration::from_millis(BASE_DELAY_MS),
        )
    }

    pub fn new_with_base_url(
        credentials_path: PathBuf,
        spoof: SpoofConfig,
        http: reqwest::Client,
        base_url: String,
    ) -> Result<Self, LlmError> {
        Self::new_inner(
            credentials_path,
            spoof,
            http,
            base_url.clone(),
            format!("{base_url}/"),
            Duration::from_millis(10),
        )
    }

    fn new_inner(
        credentials_path: PathBuf,
        spoof: SpoofConfig,
        http: reqwest::Client,
        base_url: String,
        preflight_url: String,
        retry_base_delay: Duration,
    ) -> Result<Self, LlmError> {
        validate_spoof_headers(&spoof)?;
        let oauth_client = Arc::new(CodexOAuthClient::new(http.clone()));
        Ok(Self {
            http,
            credentials_path,
            spoof,
            base_url,
            oauth_client,
            tls_preflight_cache: OnceCell::new(),
            refresh_lock: Mutex::new(()),
            preflight_url,
            retry_base_delay,
            aliases: CodexAliasMap::default(),
        })
    }

    /// Builder: attach an alias map sourced from operator config.
    pub fn with_alias_map(mut self, aliases: CodexAliasMap) -> Self {
        self.aliases = aliases;
        self
    }

    pub fn resolve_url(&self) -> String {
        format!("{}/codex/responses", self.base_url.trim_end_matches('/'))
    }

    pub fn build_request_body(&self, req: &LlmRequest) -> Result<ResponsesRequest, LlmError> {
        let session_id = Uuid::new_v4().to_string();
        let input = messages_to_responses_input(req)?;
        let instructions = join_system_prompt(&req.system);
        let tools = if req.tools.is_empty() {
            None
        } else {
            Some(tools_to_responses_tools(&req.tools)?)
        };
        let reasoning = build_reasoning_config(&req.model, req.effort.as_deref());

        Ok(ResponsesRequest {
            model: req.model.clone(),
            store: false,
            stream: true,
            instructions,
            input,
            tools,
            tool_choice: Some("auto".into()),
            parallel_tool_calls: Some(true),
            temperature: None,
            reasoning,
            service_tier: None,
            text: Some(TextConfig {
                verbosity: Some("low".into()),
            }),
            include: Some(vec!["reasoning.encrypted_content".into()]),
            prompt_cache_key: Some(session_id),
        })
    }

    async fn ensure_preflight(&self) -> Result<(), LlmError> {
        let result = self
            .tls_preflight_cache
            .get_or_init(|| async {
                run_codex_tls_preflight_url(&self.http, &self.preflight_url)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await;
        result
            .as_ref()
            .map(|_| ())
            .map_err(|e| LlmError::Http(e.clone()))
    }

    async fn send_with_retry(
        &self,
        body: &ResponsesRequest,
    ) -> Result<reqwest::Response, LlmError> {
        let mut refreshed_after_401 = false;

        for attempt in 0..=MAX_RETRIES {
            let creds = ensure_codex_token_valid(&self.credentials_path, &self.oauth_client)
                .await
                .map_err(auth_to_llm_error)?;
            let session_id = Uuid::new_v4().to_string();
            let headers = build_codex_headers(&creds, &self.spoof, &session_id)?;
            let response = self
                .http
                .post(self.resolve_url())
                .headers(headers)
                .json(body)
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(e) if attempt < MAX_RETRIES => {
                    self.sleep_before_retry(attempt).await;
                    if e.is_request() {
                        continue;
                    }
                    continue;
                }
                Err(e) => return Err(LlmError::Http(e.to_string())),
            };

            let status = response.status().as_u16();
            if status < 400 {
                return Ok(response);
            }

            let body_text = response.text().await.unwrap_or_else(|e| e.to_string());
            if status == 401 && !refreshed_after_401 {
                refreshed_after_401 = true;
                self.refresh_token_once().await?;
                continue;
            }
            if status == 401 {
                return Err(map_codex_error(status, body_text, true));
            }
            if is_quota_exceeded(&body_text) {
                return Err(LlmError::QuotaExceeded(body_text));
            }
            if is_retryable(status, &body_text) && attempt < MAX_RETRIES {
                self.sleep_before_retry(attempt).await;
                continue;
            }
            return Err(map_codex_error(status, body_text, false));
        }

        Err(LlmError::Aborted)
    }

    async fn refresh_token_once(&self) -> Result<(), LlmError> {
        let _guard = self.refresh_lock.lock().await;
        let (creds, _) =
            read_codex_credentials_locked(&self.credentials_path).map_err(auth_to_llm_error)?;
        let refreshed = self
            .oauth_client
            .refresh(creds.refresh_token.expose())
            .await
            .map_err(auth_to_llm_error)?;
        write_codex_credentials_atomic(&self.credentials_path, &refreshed)
            .map_err(auth_to_llm_error)
    }

    async fn sleep_before_retry(&self, attempt: u32) {
        let delay = self.retry_base_delay * 2u32.saturating_pow(attempt);
        tokio::time::sleep(delay).await;
    }
}

#[async_trait]
impl LlmProvider for CodexProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        crate::compaction_policy::ProviderFamily::CodexOAuth
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "gpt-5.5".into(),
                display_name: "GPT-5.5".into(),
                context_window: 0,
            },
            ModelInfo {
                id: "gpt-5.4".into(),
                display_name: "GPT-5.4".into(),
                context_window: 0,
            },
            ModelInfo {
                id: "gpt-5.4-mini".into(),
                display_name: "GPT-5.4 Mini".into(),
                context_window: 0,
            },
            ModelInfo {
                id: "gpt-5.3-codex".into(),
                display_name: "GPT-5.3 Codex".into(),
                context_window: 0,
            },
        ]
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
        let response = self.send_with_retry(&body).await?;
        let byte_stream = response.bytes_stream();

        let (raw_tx, mut raw_rx) = tokio::sync::mpsc::channel(256);
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        tokio::spawn(forward_codex_sse(byte_stream, raw_tx));
        tokio::spawn(async move {
            let mut accumulator = StreamAccumulator::default();
            while let Some(item) = raw_rx.recv().await {
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
                                            message: e.to_string(),
                                        })
                                        .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent::Error {
                                error_type: "http_error".into(),
                                message: e.to_string(),
                            })
                            .await;
                        return;
                    }
                }
            }
        });
        Ok(rx)
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut rx = self.stream(request).await?;
        let mut text = String::new();
        let mut usage = Usage::default();
        let mut stop_reason = String::new();

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::MessageStart {
                    usage: start_usage, ..
                } => usage.merge(&start_usage),
                StreamEvent::TextDelta { text: delta, .. } => text.push_str(&delta),
                StreamEvent::MessageDelta {
                    usage: delta_usage,
                    stop_reason: delta_stop,
                } => {
                    if let Some(delta_usage) = delta_usage {
                        usage.merge(&delta_usage);
                    }
                    if let Some(delta_stop) = delta_stop {
                        stop_reason = delta_stop;
                    }
                }
                StreamEvent::Error { message, .. } => return Err(LlmError::Http(message)),
                _ => {}
            }
        }

        Ok(LlmResponse {
            content: if text.is_empty() {
                Vec::new()
            } else {
                vec![serde_json::json!({"type": "text", "text": text})]
            },
            usage,
            stop_reason,
        })
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

pub fn build_reasoning_config(model: &str, effort: Option<&str>) -> Option<ReasoningConfig> {
    effort.map(|effort| ReasoningConfig {
        effort: Some(clamp_reasoning_effort(model, effort)),
        summary: Some("auto".into()),
    })
}

pub fn clamp_reasoning_effort(model_id: &str, effort: &str) -> String {
    let id = model_id.rsplit('/').next().unwrap_or(model_id);
    if (id.starts_with("gpt-5.2")
        || id.starts_with("gpt-5.3")
        || id.starts_with("gpt-5.4")
        || id.starts_with("gpt-5.5"))
        && effort == "minimal"
    {
        return "low".into();
    }
    if id == "gpt-5.1" && effort == "xhigh" {
        return "high".into();
    }
    if id == "gpt-5.3-codex-mini" {
        if effort == "high" || effort == "xhigh" {
            return "high".into();
        }
        return "medium".into();
    }
    effort.into()
}

pub fn build_codex_headers(
    creds: &CodexCredentials,
    spoof: &SpoofConfig,
    session_id: &str,
) -> Result<HeaderMap, LlmError> {
    let mut headers = HeaderMap::new();
    insert_header(
        &mut headers,
        "Authorization",
        &format!("Bearer {}", creds.access_token.expose()),
    )?;
    insert_header(&mut headers, "chatgpt-account-id", &creds.account_id)?;
    insert_header(&mut headers, "originator", &spoof.originator)?;
    insert_header(&mut headers, "User-Agent", &spoof.user_agent)?;
    insert_header(&mut headers, "OpenAI-Beta", &spoof.openai_beta)?;
    insert_header(&mut headers, "accept", "text/event-stream")?;
    insert_header(&mut headers, "content-type", "application/json")?;
    insert_header(&mut headers, "session_id", session_id)?;
    insert_header(&mut headers, "x-client-request-id", session_id)?;

    for (key, value) in &spoof.extra_headers {
        insert_header(&mut headers, key, value)?;
    }
    Ok(headers)
}

pub fn validate_spoof_headers(spoof: &SpoofConfig) -> Result<(), LlmError> {
    for key in spoof.extra_headers.keys() {
        let lower = key.to_lowercase();
        if RESERVED_HEADERS.contains(&lower.as_str()) {
            return Err(LlmError::Auth(format!(
                "spoof.extra_headers contains reserved header: {key}"
            )));
        }
    }
    Ok(())
}

fn insert_header(headers: &mut HeaderMap, key: &str, value: &str) -> Result<(), LlmError> {
    let name = HeaderName::from_bytes(key.as_bytes())
        .map_err(|e| LlmError::Serialize(format!("invalid header name `{key}`: {e}")))?;
    let value = HeaderValue::from_str(value)
        .map_err(|e| LlmError::Serialize(format!("invalid header value for `{key}`: {e}")))?;
    headers.insert(name, value);
    Ok(())
}

fn is_retryable(status: u16, body: &str) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
        || contains_any(
            body,
            &[
                "rate limit",
                "rate-limit",
                "ratelimit",
                "overloaded",
                "service unavailable",
                "upstream connect",
                "connection refused",
            ],
        )
}

fn is_quota_exceeded(body: &str) -> bool {
    body.to_lowercase().contains("usage limit")
}

fn contains_any(body: &str, needles: &[&str]) -> bool {
    let body = body.to_lowercase();
    needles.iter().any(|needle| body.contains(needle))
}

fn map_codex_error(status: u16, body: String, refreshed: bool) -> LlmError {
    if status == 401 && body.to_lowercase().contains("originator mismatch") {
        return LlmError::Auth(
            "Codex spoof manifest outdated, run `archon update-codex-compat`".into(),
        );
    }
    match status {
        401 if refreshed => LlmError::Auth("Codex token rejected after refresh".into()),
        401 | 403 => LlmError::Auth(body),
        429 if is_quota_exceeded(&body) => LlmError::QuotaExceeded(body),
        429 => LlmError::RateLimited {
            retry_after_secs: 1,
        },
        500..=599 => LlmError::Server {
            status,
            message: body,
        },
        _ => LlmError::Http(format!("status {status}: {body}")),
    }
}

fn auth_to_llm_error(err: AuthError) -> LlmError {
    LlmError::Auth(err.to_string())
}
