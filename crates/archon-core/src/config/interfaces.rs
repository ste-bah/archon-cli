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
    /// Read replies aloud. Independent of `enabled`: speaking and listening are
    /// separate devices and separate wants.
    pub speak: bool,
    /// `"kokoro"` (default) or `"openai"`. Both talk `/v1/audio/speech`.
    pub tts_provider: String,
    /// Where the speech endpoint lives. The default is a local `kokoro-fastapi`.
    pub tts_url: String,
    /// Model name the endpoint expects.
    pub tts_model: String,
    /// Voice name. Kokoro ships `af_heart`, `af_bella`, `am_michael` and more.
    pub tts_voice: String,
    /// Only needed by a hosted endpoint; a local Kokoro wants none.
    pub tts_api_key: String,
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
            speak: false,
            // Kokoro-82M by default: it is a neural model and the voices sound
            // like people. The formant and concatenative synthesisers are
            // cheaper and are not worth listening to.
            tts_provider: "kokoro".into(),
            tts_url: "http://127.0.0.1:8880".into(),
            tts_model: "kokoro".into(),
            tts_voice: "af_heart".into(),
            tts_api_key: String::new(),
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
