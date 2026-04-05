pub mod auth;
pub mod protocol;
pub mod server;
pub mod ssh;
pub mod sync;
pub mod websocket;

use std::path::PathBuf;

use async_trait::async_trait;

/// How files are kept in sync between local and remote.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    #[default]
    Manual,
    Auto,
}

/// Connection parameters for a remote SSH agent session.
#[derive(Debug, Clone)]
pub struct SshConnectionConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_file: Option<PathBuf>,
    pub agent_forwarding: bool,
    pub session_id: String,
    pub sync_mode: SyncMode,
}

/// An established remote session.
pub struct RemoteSession {
    pub session_id: String,
    // Used by SSH and WebSocket transport implementations to send/recv messages.
    #[allow(dead_code)]
    pub(crate) inner: Box<dyn RemoteSessionInner + Send + Sync>,
}

impl std::fmt::Debug for RemoteSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSession")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
#[allow(dead_code)]
pub(crate) trait RemoteSessionInner {
    async fn send(&self, message: &protocol::AgentMessage) -> anyhow::Result<()>;
    async fn recv(&self) -> anyhow::Result<protocol::AgentMessage>;
    async fn disconnect(&self) -> anyhow::Result<()>;
}

#[async_trait]
pub trait RemoteTransport: Send + Sync {
    async fn connect(&self, config: &SshConnectionConfig) -> anyhow::Result<RemoteSession>;
}
