//! Model call (FCDP drafting + judge gates) + model-resolution seam.
//!
//! Routes through archon-llm's `AnthropicClient` so BOTH auth paths work:
//!   * **Subscription (OAuth)** — resolved from `archon login` credentials / OAuth env
//!     tokens; archon-llm forces the mandatory Claude-Code identity + betas (Anthropic
//!     gates subscription API access to Claude-Code-identity traffic).
//!   * **API key** — `ANTHROPIC_API_KEY` / `ARCHON_API_KEY`; clean identity, no spoof.
//!
//! Auth + identity are resolved by archon's own `resolve_auth_from_env` +
//! `resolve_identity_mode`, so this matches how archon itself authenticates. The request
//! carries the FCDP-critical knobs — adaptive thinking + `effort:medium` (the L6–L8
//! learning) — via `MessageRequest`; effort reaches the wire thanks to the
//! `supports_output_effort` fix in archon-llm. The async streaming client is driven from
//! a single blocking bridge so the sequential orchestrator stays synchronous.

use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::auth::resolve_auth_from_env;
use archon_llm::identity::{
    get_or_create_device_id, resolve_identity_mode, IdentityConfigView, IdentityProvider,
};
use archon_llm::streaming::StreamEvent;
use serde_json::{json, Value};

/// Committed/public default (guaranteed fallback since public Fable access is uncertain).
/// Local runs override to `claude-fable-5` via config/CLI (see `resolve_model`).
pub const DEFAULT_MODEL: &str = "claude-opus-4-8";

/// Central model-resolution seam: `--model` override → archon config model → default.
///
/// `config_model` is the value the archon system persists (`LlmConfig.model`, home of the
/// `/model` selection). The standalone binary passes it from the environment; follow-up #1
/// passes the in-process session model here — a one-line hookup.
pub fn resolve_model(cli_override: Option<&str>, config_model: Option<&str>) -> String {
    cli_override
        .or(config_model)
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

/// Build the MessageRequest — the FCDP contract: adaptive thinking, effort:medium, a single
/// user message, no system/tools. This is the V0 parity target (fields the pipeline was
/// validated on). archon-llm adds transport concerns (stream flag, identity/betas) on top.
pub fn build_message_request(model: &str, max_tokens: u32, prompt: &str) -> MessageRequest {
    MessageRequest {
        model: model.to_string(),
        max_tokens,
        system: vec![],
        messages: vec![json!({"role": "user", "content": prompt})],
        tools: vec![],
        thinking: Some(json!({"type": "adaptive"})),
        speed: None,
        effort: Some("medium".to_string()),
        request_origin: Some("fcdp".to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct FableResponse {
    pub text: String,
    pub usage: Value,
    pub stop_reason: Option<String>,
}

/// Errors that abort a model call (fidelity: never gate an empty draft).
#[derive(Debug)]
pub enum FableError {
    /// Client construction, auth resolution, or transport/API failure.
    Client(String),
    /// No text content came back (thinking-only / truncation) — must not be gated.
    Empty { stop_reason: Option<String> },
}

impl std::fmt::Display for FableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FableError::Client(e) => write!(f, "model client error: {e}"),
            FableError::Empty { stop_reason } => {
                write!(f, "empty model output (stop_reason={stop_reason:?})")
            }
        }
    }
}
impl std::error::Error for FableError {}

/// A ready model client: an archon-llm `AnthropicClient` (auth+identity resolved from env)
/// plus a Tokio runtime to drive its async streaming API from synchronous call sites.
pub struct FableClient {
    client: AnthropicClient,
    rt: tokio::runtime::Runtime,
}

impl FableClient {
    /// Build from the environment: subscription (OAuth credentials / tokens) or API key,
    /// exactly as archon resolves auth. OAuth → Claude-Code identity (required); key → clean.
    pub fn from_env() -> Result<Self, FableError> {
        let auth = resolve_auth_from_env().map_err(|e| FableError::Client(e.to_string()))?;
        let mode = resolve_identity_mode(&auth, false, &IdentityConfigView::default());
        let identity = IdentityProvider::new(
            mode,
            uuid::Uuid::new_v4().to_string(),
            get_or_create_device_id(),
            String::new(),
        );
        let client = AnthropicClient::new(auth, identity, None);
        let rt = tokio::runtime::Runtime::new().map_err(|e| FableError::Client(e.to_string()))?;
        Ok(Self { client, rt })
    }

    /// One model call. Concatenates TextDelta content (thinking deltas ignored, matching
    /// Python's `b.get("text","")`); hard-fails on empty output.
    pub fn call(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
    ) -> Result<FableResponse, FableError> {
        let request = build_message_request(model, max_tokens, prompt);
        self.rt.block_on(async {
            let mut rx = self
                .client
                .stream_message(request)
                .await
                .map_err(|e| FableError::Client(e.to_string()))?;

            let mut text = String::new();
            let mut stop_reason: Option<String> = None;
            let mut input_tokens: u64 = 0;
            let mut output_tokens: u64 = 0;

            while let Some(ev) = rx.recv().await {
                match ev {
                    StreamEvent::TextDelta { text: t, .. } => text.push_str(&t),
                    StreamEvent::MessageStart { usage, .. } => {
                        input_tokens = usage.input_tokens;
                    }
                    StreamEvent::MessageDelta {
                        stop_reason: sr,
                        usage,
                    } => {
                        if sr.is_some() {
                            stop_reason = sr;
                        }
                        if let Some(u) = usage {
                            output_tokens = u.output_tokens;
                        }
                    }
                    StreamEvent::Error { error_type, .. } => {
                        return Err(FableError::Client(format!("stream error: {error_type}")));
                    }
                    _ => {}
                }
            }

            if text.trim().is_empty() {
                return Err(FableError::Empty { stop_reason });
            }
            Ok(FableResponse {
                text,
                usage: json!({"input_tokens": input_tokens, "output_tokens": output_tokens}),
                stop_reason,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_model_precedence() {
        assert_eq!(resolve_model(Some("m-cli"), Some("m-cfg")), "m-cli");
        assert_eq!(resolve_model(None, Some("m-cfg")), "m-cfg");
        assert_eq!(resolve_model(None, None), DEFAULT_MODEL);
        assert_eq!(resolve_model(None, None), "claude-opus-4-8");
    }

    #[test]
    fn v0_message_request_fields() {
        // The FCDP contract: adaptive thinking, effort:medium, one user message, no system/tools.
        let r = build_message_request("claude-fable-5", 8000, "hello");
        assert_eq!(r.model, "claude-fable-5");
        assert_eq!(r.max_tokens, 8000);
        assert_eq!(r.thinking, Some(json!({"type": "adaptive"})));
        assert_eq!(r.effort.as_deref(), Some("medium"));
        assert!(r.system.is_empty() && r.tools.is_empty());
        assert_eq!(
            r.messages,
            vec![json!({"role": "user", "content": "hello"})]
        );
    }

    #[test]
    fn v0_effort_reaches_the_wire_body() {
        // Regression against the archon-llm stub: the serialized body for a fable/opus model
        // MUST include output_config.effort=medium + thinking.type=adaptive (API-key path,
        // clean identity → no spoof blocks to complicate the assertion).
        use archon_llm::auth::AuthProvider;
        use archon_llm::identity::{IdentityMode, IdentityProvider};
        use archon_llm::types::Secret;
        let client = AnthropicClient::new(
            AuthProvider::ApiKey(Secret::new("test-key".to_string())),
            IdentityProvider::new(IdentityMode::Clean, "s".into(), "d".into(), String::new()),
            None,
        );
        for model in ["claude-fable-5", "claude-opus-4-8"] {
            let body = client
                .build_request_body(&build_message_request(model, 8000, "hi"))
                .unwrap();
            let v: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["model"], model);
            assert_eq!(v["max_tokens"], 8000);
            assert_eq!(v["thinking"], json!({"type": "adaptive"}));
            assert_eq!(
                v["output_config"],
                json!({"effort": "medium"}),
                "effort must reach the wire for {model}"
            );
            assert_eq!(v["messages"][0]["content"], "hi");
        }
    }
}
