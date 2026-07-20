use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReflexionConfig {
    pub enabled: bool,
    pub max_per_agent: usize,
}

impl Default for ReflexionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_per_agent: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CostConfig {
    pub warn_threshold: f64,
    pub hard_limit: f64,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            warn_threshold: 5.0,
            hard_limit: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub max_files: u32,
    pub max_file_size_mb: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            max_files: 50,
            max_file_size_mb: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    pub db_path: Option<String>,
    pub auto_resume: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            db_path: None,
            auto_resume: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointConfig {
    pub enabled: bool,
    pub max_checkpoints: u32,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_checkpoints: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsciousnessConfig {
    /// Whether the inner voice feature is enabled.
    pub inner_voice: bool,
    /// Energy decay rate applied each turn (multiplied by current energy).
    pub energy_decay_rate: f32,
    /// Energy regenerated after successful tool use.
    pub energy_regen_rate: f32,
    /// Minimum active-session energy.
    pub energy_floor: f32,
    /// Behavioral rules to seed into the memory graph on startup.
    /// If non-empty, these replace the built-in defaults.
    /// Idempotent: rules already present are not duplicated.
    /// Maximum 50 rules. Each must be a non-empty string.
    pub initial_rules: Vec<String>,
    /// Whether to persist personality state (InnerVoice + rule scores) across sessions.
    pub persist_personality: bool,
    /// Maximum number of personality snapshots to retain (oldest pruned first).
    pub personality_history_limit: u32,
}

impl Default for ConsciousnessConfig {
    fn default() -> Self {
        Self {
            inner_voice: true,
            energy_decay_rate: 0.98,
            energy_regen_rate: 0.005,
            energy_floor: 0.1,
            initial_rules: Vec::new(),
            persist_personality: true,
            personality_history_limit: 50,
        }
    }
}

/// TUI-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Enable vim-style keybindings in the input area. Default: `false`.
    pub vim_mode: bool,
    /// Default verbosity mode.  `true` = verbose (show everything), `false` = brief.
    /// Can be overridden per-session via the VerbosityToggle tool or `Ctrl+V`.
    pub verbose: bool,
    /// Named color theme.  Built-ins: intj, intp, ..., dark, light, ocean, fire,
    /// forest, mono, daltonized, auto.  Unknown names fall back to `"dark"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            vim_mode: false,
            verbose: true,
            theme: None,
        }
    }
}
