//! One-shot `query()` function for archon-sdk (TASK-CLI-305).

use std::path::PathBuf;
use std::sync::Arc;

use tokio_stream::wrappers::ReceiverStream;

use crate::error::SdkError;
use crate::messages::{SdkMessage, SdkResultMessage, SdkUsage};
use crate::{SdkAuth, SdkMcpServer, SdkStream};

// ── SdkOptions ────────────────────────────────────────────────────────────────

/// Configuration for a single [`query`] call.
#[derive(Debug, Clone)]
pub struct SdkOptions {
    /// Authentication method. Defaults to [`SdkAuth::FromEnv`].
    pub auth: SdkAuth,
    /// Model to use. Defaults to `claude-sonnet-4-6`.
    pub model: String,
    /// Maximum tokens to generate per turn. Defaults to `8192`.
    pub max_tokens: u32,
    /// Optional system prompt prepended to every conversation.
    pub system_prompt: Option<String>,
    /// Optional working directory for tool operations.
    pub cwd: Option<PathBuf>,
    /// Optional in-process MCP server providing tool definitions.
    pub mcp_server: Option<Arc<SdkMcpServer>>,
    /// Sessions directory override for persistence helpers.
    pub sessions_dir: Option<PathBuf>,
    /// Thinking configuration (set by builder; `None` means no thinking block sent).
    pub thinking: Option<crate::builder::ThinkingConfig>,
}

impl Default for SdkOptions {
    fn default() -> Self {
        Self {
            auth: SdkAuth::FromEnv,
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 8192,
            system_prompt: None,
            cwd: None,
            mcp_server: None,
            sessions_dir: None,
            thinking: None,
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run a one-shot conversation and stream the response.
///
/// Returns a [`SdkStream`] that yields [`SdkMessage`] items. The stream always
/// ends with either a [`SdkMessage::ResultMessage`] or an `Err(SdkError)`.
///
/// # Example
///
/// ```rust,no_run
/// use futures_util::StreamExt;
/// use archon_sdk::{query, SdkOptions};
///
/// # #[tokio::main]
/// # async fn main() {
/// let mut stream = query("What is 2 + 2?", SdkOptions::default());
/// while let Some(item) = stream.next().await {
///     match item {
///         Ok(msg) => println!("{msg:?}"),
///         Err(e) => eprintln!("error: {e}"),
///     }
/// }
/// # }
/// ```
pub fn query(prompt: impl Into<String>, options: SdkOptions) -> SdkStream {
    query_internal(prompt.into(), options, None)
}

// ── Internal implementation ───────────────────────────────────────────────────

/// Internal query that optionally records messages to a session.
pub(crate) fn query_internal(
    prompt: String,
    options: SdkOptions,
    _session: Option<(String, PathBuf)>,
) -> SdkStream {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SdkMessage, SdkError>>(64);

    tokio::spawn(async move {
        if let Err(e) = run_query(&prompt, &options, &tx).await {
            let _ = tx.send(Err(e)).await;
        }
    });

    Box::pin(ReceiverStream::new(rx))
}

async fn run_query(
    prompt: &str,
    options: &SdkOptions,
    tx: &tokio::sync::mpsc::Sender<Result<SdkMessage, SdkError>>,
) -> Result<(), SdkError> {
    use archon_llm::anthropic::{AnthropicClient, MessageRequest};
    use archon_llm::identity::{IdentityMode, IdentityProvider};
    use archon_llm::streaming::StreamEvent;
    use archon_llm::types::ContentBlockType;

    // Resolve auth — fail early so error is surfaced as first stream item
    let auth = build_auth_provider(&options.auth)?;

    let identity = IdentityProvider::new(
        IdentityMode::Clean,
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        String::new(),
    );

    let client = AnthropicClient::new(auth, identity, None);

    // Build request
    let mut request = MessageRequest {
        model: options.model.clone(),
        max_tokens: options.max_tokens,
        ..Default::default()
    };

    if let Some(ref sys) = options.system_prompt {
        request.system = vec![serde_json::json!({ "type": "text", "text": sys })];
    }

    request.messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt
    })];

    if let Some(ref mcp) = options.mcp_server {
        request.tools = mcp.tool_schemas();
    }

    // Apply thinking configuration
    if let Some(ref thinking) = options.thinking {
        use crate::builder::ThinkingConfig;
        request.thinking = match thinking {
            ThinkingConfig::Enabled { budget_tokens } => {
                Some(serde_json::json!({ "type": "enabled", "budget_tokens": budget_tokens }))
            }
            ThinkingConfig::Auto => Some(serde_json::json!({ "type": "auto" })),
            ThinkingConfig::Disabled => None,
        };
    }

    // Stream response
    let mut receiver = client
        .stream_message(request)
        .await
        .map_err(|e| SdkError::Api {
            status: 0,
            message: e.to_string(),
        })?;

    let mut text_buf = String::new();
    let mut usage = SdkUsage::default();
    let mut stop_reason = "end_turn".to_string();

    while let Some(event) = receiver.recv().await {
        match event {
            StreamEvent::MessageStart { usage: u, .. } => {
                usage.input_tokens += u.input_tokens;
            }
            StreamEvent::ContentBlockStart {
                block_type,
                tool_use_id: _,
                tool_name: _,
                ..
            } => {
                if block_type == ContentBlockType::Text {
                    text_buf.clear();
                }
            }
            StreamEvent::TextDelta { text, .. } => {
                text_buf.push_str(&text);
            }
            StreamEvent::ContentBlockStop { .. } => {
                if !text_buf.is_empty() {
                    let _ = tx
                        .send(Ok(SdkMessage::AssistantMessage {
                            content: std::mem::take(&mut text_buf),
                        }))
                        .await;
                }
            }
            StreamEvent::MessageDelta {
                stop_reason: sr,
                usage: u,
            } => {
                if let Some(sr) = sr {
                    stop_reason = sr;
                }
                if let Some(u) = u {
                    usage.output_tokens += u.output_tokens;
                }
            }
            StreamEvent::MessageStop => break,
            StreamEvent::Error { message, .. } => {
                return Err(SdkError::Api { status: 0, message });
            }
            _ => {}
        }
    }

    let _ = tx
        .send(Ok(SdkMessage::ResultMessage(SdkResultMessage {
            stop_reason,
            usage,
        })))
        .await;

    Ok(())
}

// ── Auth helpers ──────────────────────────────────────────────────────────────

fn build_auth_provider(auth: &SdkAuth) -> Result<archon_llm::auth::AuthProvider, SdkError> {
    use archon_llm::auth::AuthProvider;
    use archon_llm::types::Secret;

    match auth {
        SdkAuth::FromEnv => {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| SdkError::Auth("ANTHROPIC_API_KEY not set".to_string()))?;
            if key.is_empty() {
                return Err(SdkError::Auth("ANTHROPIC_API_KEY is empty".to_string()));
            }
            Ok(AuthProvider::ApiKey(Secret::new(key)))
        }
        SdkAuth::ApiKey(key) => {
            if key.is_empty() {
                return Err(SdkError::Auth("API key is empty".to_string()));
            }
            Ok(AuthProvider::ApiKey(Secret::new(key.clone())))
        }
        SdkAuth::BearerToken(token) => Ok(AuthProvider::BearerToken(Secret::new(token.clone()))),
    }
}
