//! Builder pattern API for archon-sdk (TASK-CLI-306).
//!
//! Provides fluent builder structs for constructing agent queries and sessions
//! with ergonomic method chaining and validation at `build()` time.

use std::path::PathBuf;
use std::sync::Arc;

use crate::error::SdkError;
use crate::mcp_server::SdkMcpServer;
use crate::{SdkAuth, SdkStream};

// ── ThinkingConfig ────────────────────────────────────────────────────────────

/// Controls how the model uses extended thinking (chain-of-thought).
///
/// Matches the Anthropic API's thinking parameter model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingConfig {
    /// Extended thinking is disabled (default).
    Disabled,
    /// Extended thinking is enabled with a token budget for thinking blocks.
    Enabled {
        /// Maximum tokens the model may use for thinking.
        budget_tokens: u32,
    },
    /// The model adaptively decides when to use extended thinking.
    Auto,
}

// ── PermissionMode ────────────────────────────────────────────────────────────

/// Controls how tool-use permissions are evaluated during a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionMode {
    /// Automatically apply safe defaults (default).
    Auto,
}

// ── AgentOptions ──────────────────────────────────────────────────────────────

/// Fully-validated agent configuration produced by [`AgentBuilder::build`].
///
/// `AgentOptions` is `Clone` — pass it to multiple [`AgentQuery`] instances
/// for concurrent runs with the same configuration.
#[derive(Debug, Clone)]
pub struct AgentOptions {
    /// Model identifier.
    pub model: String,
    /// Authentication credentials.
    pub auth: SdkAuth,
    /// Optional system prompt.
    pub system_prompt: Option<String>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Thinking configuration.
    pub thinking: ThinkingConfig,
    /// Tool permission mode.
    pub permission_mode: PermissionMode,
    /// Optional in-process MCP server providing tools.
    pub mcp_server: Option<Arc<SdkMcpServer>>,
    /// Optional working directory for tool operations.
    pub cwd: Option<PathBuf>,
}

// ── AgentQuery ────────────────────────────────────────────────────────────────

/// A configured agent query handle produced by [`AgentBuilder::build`].
///
/// Call `.run(prompt)` to start a conversation and receive a stream.
#[derive(Debug)]
pub struct AgentQuery {
    options: AgentOptions,
}

impl AgentQuery {
    /// The resolved agent options (useful for cloning or inspection).
    pub fn options(&self) -> &AgentOptions {
        &self.options
    }

    /// Run a one-shot conversation with `prompt` and return a stream of messages.
    pub fn run(&self, prompt: impl Into<String>) -> SdkStream {
        let sdk_opts = self.to_sdk_options();
        crate::query::query_internal(prompt.into(), sdk_opts, None)
    }

    fn to_sdk_options(&self) -> crate::query::SdkOptions {
        crate::query::SdkOptions {
            auth: self.options.auth.clone(),
            model: self.options.model.clone(),
            max_tokens: self.options.max_tokens,
            system_prompt: self.options.system_prompt.clone(),
            cwd: self.options.cwd.clone(),
            mcp_server: self.options.mcp_server.clone(),
            sessions_dir: None,
            thinking: Some(self.options.thinking.clone()),
        }
    }
}

// ── AgentBuilder ──────────────────────────────────────────────────────────────

/// Fluent builder for constructing an [`AgentQuery`].
///
/// # Required fields
/// - Auth: call `.api_key()` or `.bearer_token()`
///
/// # Example
///
/// ```rust,no_run
/// use archon_sdk::builder::{AgentBuilder, ThinkingConfig, PermissionMode};
/// use archon_sdk::create_sdk_mcp_server;
///
/// # fn main() -> Result<(), archon_sdk::SdkError> {
/// let query = AgentBuilder::new()
///     .model("claude-sonnet-4-6")
///     .api_key("sk-ant-...")
///     .system_prompt("You are a helpful assistant.")
///     .max_tokens(4096)
///     .thinking(ThinkingConfig::Enabled { budget_tokens: 8192 })
///     .permission_mode(PermissionMode::Auto)
///     .build()?;
///
/// // let stream = query.run("Write me a function");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct AgentBuilder {
    model: Option<String>,
    auth: Option<SdkAuth>,
    system_prompt: Option<String>,
    max_tokens: Option<u32>,
    thinking: Option<ThinkingConfig>,
    permission_mode: Option<PermissionMode>,
    mcp_server: Option<Arc<SdkMcpServer>>,
    cwd: Option<PathBuf>,
}

impl AgentBuilder {
    /// Create a new builder with no fields set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the model identifier. Defaults to `claude-sonnet-4-6`.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set an API key for authentication.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.auth = Some(SdkAuth::ApiKey(key.into()));
        self
    }

    /// Set an OAuth bearer token for authentication.
    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth = Some(SdkAuth::BearerToken(token.into()));
        self
    }

    /// Set the system prompt.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set the maximum tokens to generate. Defaults to `8096`.
    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }

    /// Set the thinking configuration. Defaults to `ThinkingConfig::Disabled`.
    pub fn thinking(mut self, config: ThinkingConfig) -> Self {
        self.thinking = Some(config);
        self
    }

    /// Set the permission mode. Defaults to `PermissionMode::Auto`.
    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    /// Register an in-process MCP server providing tools.
    pub fn tool(mut self, server: SdkMcpServer) -> Self {
        self.mcp_server = Some(Arc::new(server));
        self
    }

    /// Set the working directory for tool operations.
    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = Some(path.into());
        self
    }

    /// Validate fields and produce an [`AgentQuery`].
    ///
    /// # Errors
    ///
    /// - [`SdkError::MissingApiKey`] — no auth was provided
    /// - [`SdkError::MissingModel`] — model was set to an empty string
    pub fn build(self) -> Result<AgentQuery, SdkError> {
        let auth = self.auth.ok_or(SdkError::MissingApiKey)?;

        // Reject empty API key
        if let SdkAuth::ApiKey(ref key) = auth {
            if key.is_empty() {
                return Err(SdkError::MissingApiKey);
            }
        }

        let model = self
            .model
            .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
        if model.is_empty() {
            return Err(SdkError::MissingModel);
        }

        Ok(AgentQuery {
            options: AgentOptions {
                model,
                auth,
                system_prompt: self.system_prompt,
                max_tokens: self.max_tokens.unwrap_or(8096),
                thinking: self.thinking.unwrap_or(ThinkingConfig::Disabled),
                permission_mode: self.permission_mode.unwrap_or(PermissionMode::Auto),
                mcp_server: self.mcp_server,
                cwd: self.cwd,
            },
        })
    }
}

// ── SessionHandle ─────────────────────────────────────────────────────────────

/// A configured multi-turn session handle produced by [`SessionBuilder::build`].
pub struct SessionHandle {
    model: String,
    auth: SdkAuth,
    system_prompt: Option<String>,
    cwd: Option<PathBuf>,
}

impl SessionHandle {
    /// Send a message and return a stream of response messages.
    pub fn send(&self, prompt: impl Into<String>) -> SdkStream {
        let opts = crate::query::SdkOptions {
            auth: self.auth.clone(),
            model: self.model.clone(),
            max_tokens: 8192,
            system_prompt: self.system_prompt.clone(),
            cwd: self.cwd.clone(),
            mcp_server: None,
            sessions_dir: None,
            thinking: None,
        };
        crate::query::query_internal(prompt.into(), opts, None)
    }
}

// ── SessionBuilder ────────────────────────────────────────────────────────────

/// Fluent builder for constructing a multi-turn [`SessionHandle`].
///
/// # Required fields
/// - Auth: call `.api_key()` or `.bearer_token()`
///
/// # Example
///
/// ```rust,no_run
/// use archon_sdk::builder::SessionBuilder;
///
/// # fn main() -> Result<(), archon_sdk::SdkError> {
/// let session = SessionBuilder::new()
///     .model("claude-sonnet-4-6")
///     .api_key("sk-ant-...")
///     .cwd("/path/to/project")
///     .build()?;
///
/// // let stream = session.send("First message");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct SessionBuilder {
    model: Option<String>,
    auth: Option<SdkAuth>,
    system_prompt: Option<String>,
    cwd: Option<PathBuf>,
}

impl SessionBuilder {
    /// Create a new session builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the model identifier. Defaults to `claude-sonnet-4-6`.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set an API key for authentication.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.auth = Some(SdkAuth::ApiKey(key.into()));
        self
    }

    /// Set an OAuth bearer token for authentication.
    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth = Some(SdkAuth::BearerToken(token.into()));
        self
    }

    /// Set the system prompt for this session.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set the working directory for tool operations.
    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = Some(path.into());
        self
    }

    /// Validate fields and produce a [`SessionHandle`].
    ///
    /// # Errors
    ///
    /// - [`SdkError::MissingApiKey`] — no auth was provided
    pub fn build(self) -> Result<SessionHandle, SdkError> {
        let auth = self.auth.ok_or(SdkError::MissingApiKey)?;
        if let SdkAuth::ApiKey(ref key) = auth {
            if key.is_empty() {
                return Err(SdkError::MissingApiKey);
            }
        }
        Ok(SessionHandle {
            model: self
                .model
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
            auth,
            system_prompt: self.system_prompt,
            cwd: self.cwd,
        })
    }
}
