use serde::{Deserialize, Serialize};

mod generated_shape;
mod generated_tuning;
mod interfaces;
mod io;
mod learning;
// Inherent impl on `MemoryConfig` only — nothing to re-export.
mod memory_open;
mod providers;
mod runtime;
mod sections;
mod topology;
mod validation;
mod world_model;

pub use generated_shape::*;
pub use generated_tuning::*;
pub use interfaces::*;
pub use io::*;
pub use learning::*;
pub use providers::*;
pub use runtime::*;
pub use sections::*;
pub use topology::*;
pub use validation::*;
pub use world_model::*;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("config parse error: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("config validation error: {0}")]
    ValidationError(String),
}

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ArchonConfig {
    pub api: ApiConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    /// Provider-namespaced model aliases. Bumping a default model only
    /// requires editing one entry here; agent code uses aliases.
    #[serde(default)]
    pub models: ModelsConfig,
    pub identity: IdentityConfig,
    pub tools: ToolsConfig,
    pub permissions: PermissionsConfig,
    pub context: ContextConfig,
    pub memory: MemoryConfig,
    pub learning: LearningConfig,
    pub cost: CostConfig,
    pub logging: LoggingConfig,
    pub session: SessionConfig,
    pub checkpoint: CheckpointConfig,
    /// Repository code index (LEANN).
    #[serde(default)]
    pub code_index: CodeIndexConfig,
    pub personality: archon_consciousness::personality::PersonalityProfile,
    pub consciousness: ConsciousnessConfig,
    pub tui: TuiConfig,
    /// Active output style name.  Resolved at startup against the
    /// `OutputStyleRegistry`.  Unknown values fall back to `"default"` with a
    /// warning.  Can be overridden by `--output-style` CLI flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,
    /// Self-update configuration.
    pub update: crate::update::UpdateConfig,
    /// Remote agent / SSH configuration.
    pub remote: SshRemoteConfig,
    /// WebSocket server configuration.
    #[serde(default)]
    pub ws_remote: WsRemoteConfig,
    /// Multi-agent orchestration configuration.
    #[serde(default)]
    pub orchestrator: crate::orchestrator::config::OrchestratorConfig,
    /// Voice input configuration.
    #[serde(default)]
    pub voice: VoiceConfig,
    /// Web UI configuration.
    #[serde(default)]
    pub web: WebConfig,
    /// Sandbox backend configuration.
    #[serde(default)]
    pub sandbox: crate::sandbox::SandboxConfig,
    /// Subagent execution configuration (authoritative fan-out concurrency cap).
    #[serde(default)]
    pub subagent: SubagentConfig,
    /// Workflow runtime configuration.
    #[serde(default)]
    pub workflow: WorkflowRuntimeConfig,
    /// Milestone 3 topology guardrail admission.
    #[serde(default)]
    pub topology: TopologyConfig,
}

#[cfg(test)]
mod tests;
