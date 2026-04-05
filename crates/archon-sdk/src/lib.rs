//! archon-sdk — embeddable library for Archon agent capabilities (TASK-CLI-305).
//!
//! Allows external Rust programs to use Archon's agent capabilities without
//! running the full CLI/TUI. Suitable for IDE extensions, web UI mode, and
//! custom agent applications.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use futures_util::StreamExt;
//! use archon_sdk::{query, SdkOptions};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let mut stream = query("What is 2 + 2?", SdkOptions::default());
//! while let Some(item) = stream.next().await {
//!     match item {
//!         Ok(msg) => println!("{msg:?}"),
//!         Err(e) => eprintln!("error: {e}"),
//!     }
//! }
//! # }
//! ```

pub mod builder;
pub mod error;
pub mod ide;
pub mod mcp_server;
pub mod messages;
pub mod query;
pub mod session;
pub mod web;

// Re-exports — public API surface

pub use builder::{AgentBuilder, AgentOptions, PermissionMode, SessionBuilder, ThinkingConfig};
pub use error::SdkError;
pub use mcp_server::{SdkMcpServer, SdkTool, create_sdk_mcp_server};
pub use messages::{SdkMessage, SdkResultMessage, SdkUsage};
pub use query::{SdkOptions, query};
pub use session::{
    ArchonSession, SdkSessionInfo, SessionOptions, fork_session, get_session_info, list_sessions,
    rename_session, tag_session, unstable_v2_create_session, unstable_v2_resume_session,
};

/// Authentication method for SDK operations.
///
/// This enum is `#[non_exhaustive]` — new variants may be added in future releases.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SdkAuth {
    /// Use the `ANTHROPIC_API_KEY` environment variable.
    FromEnv,
    /// Explicit API key string.
    ApiKey(String),
    /// OAuth bearer token (for remote-control scenarios; no browser flow).
    BearerToken(String),
}

/// The stream type returned by [`query`] and [`ArchonSession::send`].
///
/// Drive the stream with `while let Some(item) = stream.next().await { ... }`.
pub type SdkStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<SdkMessage, SdkError>> + Send>>;
