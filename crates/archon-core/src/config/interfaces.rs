use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub device: String,
    pub vad_threshold: f32,
    pub stt_provider: String,
    pub stt_api_key: String,
    pub stt_url: String,
    /// The key that starts a recording.
    ///
    /// Reported by `/voice` and nothing else: the TUI binding is fixed at
    /// `Ctrl+V` (`Action::VoiceHotkey`), so setting this to anything else
    /// changes what `/voice` prints and not what the keyboard does. The
    /// default matches the real binding rather than describing one that has
    /// never existed — it read `ctrl+shift+v` until #192.
    pub hotkey: String,
    pub toggle_mode: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            device: "default".into(),
            vad_threshold: 0.02,
            stt_provider: "openai".into(),
            stt_api_key: String::new(),
            stt_url: "https://api.openai.com".into(),
            hotkey: "ctrl+v".into(),
            toggle_mode: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Web UI config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Port to listen on.
    pub port: u16,
    /// Address to bind. `"127.0.0.1"` = localhost only (default).
    pub bind_address: String,
    /// Open default browser automatically after server starts.
    pub open_browser: bool,
    /// Maximum accepted HTTP request body size for mutating web APIs.
    pub max_body_bytes: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            port: 8421,
            bind_address: "127.0.0.1".to_string(),
            open_browser: true,
            max_body_bytes: 64 * 1024 * 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// Remote / SSH config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_file: Option<String>,
    pub agent_forwarding: bool,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            user: String::new(),
            key_file: None,
            agent_forwarding: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SshRemoteConfig {
    pub sync_mode: String,
    pub ssh: SshConfig,
}

/// WebSocket remote server configuration stored in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WsRemoteConfig {
    /// Port the WebSocket server listens on.
    #[serde(default = "default_ws_port")]
    pub port: u16,
    /// Path to a TLS certificate file (PEM).  `None` = no TLS.
    pub tls_cert: Option<String>,
    /// Path to a TLS private key file (PEM).  Required when `tls_cert` is set.
    pub tls_key: Option<String>,
}

fn default_ws_port() -> u16 {
    8420
}

impl Default for WsRemoteConfig {
    fn default() -> Self {
        Self {
            port: 8420,
            tls_cert: None,
            tls_key: None,
        }
    }
}

impl Default for SshRemoteConfig {
    fn default() -> Self {
        Self {
            sync_mode: "manual".to_string(),
            ssh: SshConfig::default(),
        }
    }
}
