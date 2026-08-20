use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub default_model: String,
    /// Tokens the model may spend REASONING before it answers.
    ///
    /// Goes on the wire as Anthropic `thinking.budget_tokens`. This is a
    /// different mechanism from `default_effort`, which selects a tier
    /// (low/medium/high) and on most templates injects a different system
    /// preamble — effort chooses how hard the model tries, this chooses how
    /// long it may think. Neither substitutes for the other, which is why both
    /// exist.
    pub thinking_budget: u32,
    /// Ceiling on the tokens ONE response may produce, reasoning included.
    ///
    /// `None` falls back to `thinking_budget`, which is what archon did
    /// unconditionally before this field existed — so an untouched config
    /// behaves exactly as it did.
    ///
    /// Why it needed separating: servers RESERVE max_tokens out of the context
    /// window before the prompt is placed, so sharing one number made a
    /// reasoning-depth change silently resize the prompt window. On a 262,144
    /// deployment a 65,536 budget left 179k for prompts; split, the same
    /// reasoning depth with a 16,384 answer ceiling leaves ~228k.
    ///
    /// MUST stay above `thinking_budget` — see `validate_token_budgets`.
    pub max_tokens: Option<u32>,
    pub default_effort: String,
    pub max_retries: u32,
    /// Override the Anthropic API base URL. Useful for pointing at LiteLLM,
    /// Ollama, or any other OpenAI-compatible / Anthropic-compatible proxy.
    /// Resolution priority:
    ///   1. `ANTHROPIC_BASE_URL` env var (always wins)
    ///   2. This field in config.toml
    ///   3. Hardcoded default: `https://api.anthropic.com/v1/messages`
    pub base_url: Option<String>,
    /// Repair `tool_use` content blocks that a proxy split across a named block
    /// and an unnamed continuation.
    ///
    /// Defaults ON. When it is not needed the pass never runs — a conforming
    /// stream has no unnamed `tool_use` block — so leaving it on costs nothing,
    /// while leaving it off on a proxy that splits blocks kills every agent turn
    /// that issues parallel tool calls. Switch it off to get strict Anthropic
    /// spec behaviour. See `agent::tool_block_repair`.
    pub repair_split_tool_blocks: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            default_model: "claude-sonnet-4-6".into(),
            thinking_budget: 16384,
            max_tokens: None,
            default_effort: "medium".into(),
            max_retries: 3,
            base_url: None,
            repair_split_tool_blocks: true,
        }
    }
}

#[path = "api_token_budget.rs"]
mod api_token_budget;

/// LLM provider configuration.
///
/// Controls which backend provider is active and allows provider-specific
/// settings to be set in the `[llm]` section of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// The active provider name (e.g. `"anthropic"`, `"openai-codex"`, `"openai"`, `"bedrock"`, `"vertex"`, `"local"`).
    pub provider: String,
    /// OpenAI provider settings.
    pub openai: LlmOpenAiConfig,
    /// AWS Bedrock provider settings.
    pub bedrock: LlmBedrockConfig,
    /// Google Vertex AI provider settings.
    pub vertex: LlmVertexConfig,
    /// Local / Ollama provider settings.
    pub local: LlmLocalConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            openai: LlmOpenAiConfig::default(),
            bedrock: LlmBedrockConfig::default(),
            vertex: LlmVertexConfig::default(),
            local: LlmLocalConfig::default(),
        }
    }
}

/// OpenAI provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmOpenAiConfig {
    /// API key. If `None`, resolved from `OPENAI_API_KEY` env var.
    pub api_key: Option<String>,
    /// Override the OpenAI base URL (e.g. for Azure OpenAI or a proxy).
    pub base_url: Option<String>,
    /// Default model to use.
    pub model: String,
}

impl Default for LlmOpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            model: "gpt-4o".to_string(),
        }
    }
}

/// AWS Bedrock provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmBedrockConfig {
    /// AWS region (e.g. `"us-east-1"`).
    pub region: String,
    /// Bedrock model ID (e.g. `"anthropic.claude-sonnet-4-6-v1:0"`).
    pub model_id: String,
}

impl Default for LlmBedrockConfig {
    fn default() -> Self {
        Self {
            region: "us-east-1".to_string(),
            model_id: "anthropic.claude-sonnet-4-6-v1:0".to_string(),
        }
    }
}

/// Google Vertex AI provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmVertexConfig {
    /// GCP project ID. If `None`, resolved from ADC or env var.
    pub project_id: Option<String>,
    /// GCP region (e.g. `"us-central1"`).
    pub region: String,
    /// Model name (e.g. `"claude-sonnet-4-6@20250514"`).
    pub model: String,
    /// Path to service account credentials JSON file.
    pub credentials_file: Option<String>,
}

impl Default for LlmVertexConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            region: "us-central1".to_string(),
            model: "claude-sonnet-4-6@20250514".to_string(),
            credentials_file: None,
        }
    }
}

/// Local / Ollama provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmLocalConfig {
    /// Base URL for the OpenAI-compatible local server.
    pub base_url: String,
    /// Default model name.
    pub model: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Whether to pull the model if not present (Ollama-specific).
    pub pull_if_missing: bool,
    /// #123: how `/effort` maps onto this backend's reasoning controls.
    ///
    /// Defaults to `mode = "off"`, which sends no reasoning fields — this
    /// provider also serves Ollama and llama.cpp, where an unexpected
    /// top-level field can be a hard 400. Enable it per deployment, e.g. for
    /// a vLLM server:
    ///
    /// ```toml
    /// [llm.local.reasoning]
    /// mode = "top_level"          # validated server-side; 400s on a typo
    /// effort_key = "reasoning_effort"
    /// [llm.local.reasoning.effort_map]
    /// low = "low"
    /// medium = "medium"
    /// high = "high"
    /// max = "max"
    /// ```
    pub reasoning: archon_llm::reasoning::ReasoningConfig,
}

impl Default for LlmLocalConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".to_string(),
            model: "llama3:8b".to_string(),
            timeout_secs: 300,
            pull_if_missing: true,
            reasoning: archon_llm::reasoning::ReasoningConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    pub mode: String,
    pub spoof_version: String,
    pub spoof_entrypoint: String,
    pub spoof_betas: Option<Vec<String>>,
    pub anti_distillation: bool,
    pub workload: Option<String>,
    pub custom: Option<CustomIdentityConfig>,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            mode: "clean".into(),
            spoof_version: "2.1.89".into(),
            spoof_entrypoint: "cli".into(),
            spoof_betas: None,
            anti_distillation: false,
            workload: None,
            custom: None,
        }
    }
}

impl IdentityConfig {
    pub fn as_view(&self) -> archon_llm::identity::IdentityConfigView<'_> {
        archon_llm::identity::IdentityConfigView {
            mode: &self.mode,
            spoof_version: &self.spoof_version,
            spoof_entrypoint: &self.spoof_entrypoint,
            spoof_betas: self.spoof_betas.as_deref(),
            anti_distillation: self.anti_distillation,
            workload: self.workload.as_deref(),
            custom: self.custom.as_ref().map(|custom| {
                archon_llm::identity::CustomIdentityConfigView {
                    user_agent: &custom.user_agent,
                    x_app: &custom.x_app,
                    extra_headers: custom.extra_headers.as_ref(),
                }
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomIdentityConfig {
    pub user_agent: String,
    pub x_app: String,
    pub extra_headers: Option<HashMap<String, String>>,
}

impl Default for CustomIdentityConfig {
    fn default() -> Self {
        Self {
            user_agent: "archon-cli/0.1.0".into(),
            x_app: "archon".into(),
            extra_headers: None,
        }
    }
}

// `ToolsConfig` and `CargoResourceConfig` live in `config/tools.rs` — they carry
// enough documentation between them to push this file past the 500-line ceiling,
// and the Bash-tool constructor belongs beside them rather than here.

/// Subagent execution configuration.
///
/// `max_concurrent` is the authoritative cap on how many subagents the
/// [`crate::subagent::SubagentManager`] runs concurrently. It is the single
/// source of truth for fan-out width in live (subagent-backed) workflows: the
/// fan-out scheduler clamps its semaphore to this value so overflow items wait
/// for a slot instead of being hard-rejected. Distinct from
/// `[orchestrator] max_concurrent`, which governs the separate team/orchestrator
/// agent pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SubagentConfig {
    pub max_concurrent: usize,
    /// Seconds a subagent's LLM stream may go silent before the round is
    /// abandoned.
    ///
    /// This guards against a provider that has stopped sending without closing
    /// the connection. It is not a thinking budget, and the two are easy to
    /// confuse: a reducer on a large context routinely pauses longer between
    /// stream events than a short guard allows, and the agent is then killed
    /// mid-work with everything it had done discarded. The hardcoded 120s was
    /// sixty times tighter than the `host_call_timeout_secs` stage it runs
    /// inside, and killed a live inventory reducer three turns into its work.
    pub stream_idle_timeout_secs: u64,

    /// When to isolate an agent that did not ask to be isolated (#184 M3).
    ///
    /// `overlap` by default, and the choice is about disk rather than
    /// correctness. A worktree itself is cheap — it shares `.git` and checks
    /// out working files only — but an agent *building* inside one grows a
    /// fresh `target/`, which on this workspace is gigabytes. Isolating every
    /// parallel writer would trade an invisible conflict for an invisible disk
    /// fire, so isolation is spent where two agents actually collide.
    pub auto_isolation: archon_tools::isolation::AutoIsolation,

    /// The most isolation any agent may have, however it was requested.
    ///
    /// `worktree` by default: an agent gets its own checkout but not its own
    /// build directory, and verification runs once after merge in the main
    /// tree. Raise it to `worktree-with-builds` when agents genuinely must
    /// build before their work can be reviewed, and expect the disk cost.
    pub isolation_max_tier: archon_tools::isolation::IsolationTier,
}

/// Generous enough that only a genuinely stalled provider trips it, and still
/// far inside the enclosing stage timeout.
pub const DEFAULT_STREAM_IDLE_TIMEOUT_SECS: u64 = 600;

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_concurrent: crate::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT,
            stream_idle_timeout_secs: DEFAULT_STREAM_IDLE_TIMEOUT_SECS,
            auto_isolation: archon_tools::isolation::AutoIsolation::Overlap,
            // Deliberately not the top rung. Reaching the expensive tier should
            // be an operator's decision, not something a spawn can talk itself
            // into.
            isolation_max_tier: archon_tools::isolation::IsolationTier::Worktree,
        }
    }
}

/// Workflow runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WorkflowRuntimeConfig {
    pub generated: GeneratedWorkflowConfig,
}

/// Generated workflow limits used by deterministic PRD scaffolds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneratedWorkflowConfig {
    pub max_repair_iterations: u8,
    pub max_investigation_iterations: u8,
    pub verification_branch_timeout_secs: u32,
    pub host_call_timeout_secs: u32,
    /// How many ready tasks the write fan-out dispatches concurrently.
    ///
    /// `None` — the default — means "the configured subagent concurrency",
    /// which is what every run got before this field existed. A value here is
    /// a *lower* bound on nothing and an upper bound on concurrency: the
    /// runtime clamps it into `1..=subagent_cap`, so setting it can only ever
    /// narrow a wave, never widen one past what the executor allows.
    ///
    /// Learned narrowing writes here too, via
    /// `archon_core::config::decide_fanout_width`; see that module for why the
    /// learner may only move this value downward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_wave_max_parallelism: Option<u8>,
}

impl Default for GeneratedWorkflowConfig {
    fn default() -> Self {
        Self {
            max_repair_iterations: 3,
            max_investigation_iterations: 3,
            // 4 hours. The previous 20 minutes starved verifiers relative to the
            // work they inspect: host calls get 2 hours to BUILD something, while
            // the branch that has to read the result, cross-check it against
            // registries and artifacts, and run its own tests had one sixth of
            // that. Observed live — a verifier timed out at 1200s and VOIDED an
            // already-accepted remediation, recording correct work as unresolved.
            // A verifier that cannot finish cannot fail-closed honestly; it just
            // disappears. Override per project with
            // workflow.generated.verification_branch_timeout_secs.
            verification_branch_timeout_secs: 14_400,
            host_call_timeout_secs: 7_200,
            // Unset: defer to the configured subagent concurrency. Naming a
            // number here would pin every project to one wave width regardless
            // of the executor it runs on.
            implementation_wave_max_parallelism: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsConfig {
    pub mode: String,
    pub allow_paths: Vec<String>,
    pub deny_paths: Vec<String>,
    pub safe_commands: Vec<String>,
    pub risky_commands: Vec<String>,
    pub dangerous_commands: Vec<String>,
    /// Fine-grained rules: tools/patterns that are always allowed.
    pub always_allow: Vec<archon_permissions::rules::ToolRule>,
    /// Fine-grained rules: tools/patterns that are always denied.
    pub always_deny: Vec<archon_permissions::rules::ToolRule>,
    /// Fine-grained rules: tools/patterns that always require confirmation.
    pub always_ask: Vec<archon_permissions::rules::ToolRule>,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            mode: "default".into(),
            allow_paths: Vec::new(),
            deny_paths: Vec::new(),
            safe_commands: Vec::new(),
            risky_commands: Vec::new(),
            dangerous_commands: Vec::new(),
            always_allow: Vec::new(),
            always_deny: Vec::new(),
            always_ask: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub db_path: Option<String>,
    /// Embedding provider: `auto`, `local`, or `openai`.
    pub embedding_provider: archon_memory::embedding::EmbeddingProviderKind,
    /// API root for the openai embedding provider (e.g. "http://127.0.0.1:1234/v1"
    /// for a local OpenAI-compatible proxy). None = the real OpenAI API. The
    /// ARCHON_MEMORY_EMBEDDING_BASE_URL / OPENAI_BASE_URL env vars take precedence.
    pub embedding_base_url: Option<String>,
    /// Model for the openai embedding provider. None = text-embedding-3-small.
    /// The ARCHON_MEMORY_EMBEDDING_MODEL env var takes precedence.
    pub embedding_model: Option<String>,
    /// Keyword/vector blend factor for hybrid search (0.0 = pure vector, 1.0 = pure keyword).
    pub hybrid_alpha: f32,
    /// Intra-op threads for the local embedder's ONNX session. None = a capped
    /// default. Process-wide: memory and the LEANN code index share one session,
    /// so this is not per-consumer.
    pub embedding_intra_threads: Option<usize>,
    /// Memory garden consolidation settings.
    pub garden: archon_memory::garden::GardenConfig,
    /// Auto-capture settings (regex-based memory detection at turn boundary).
    pub auto_capture: AutoCaptureConfig,
    /// Auto-extraction settings (LLM-driven fact extraction every N turns).
    pub auto_extraction: AutoExtractionConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: None,
            embedding_provider: archon_memory::embedding::EmbeddingProviderKind::Auto,
            embedding_base_url: None,
            embedding_model: None,
            hybrid_alpha: 0.3,
            embedding_intra_threads: None,
            garden: archon_memory::garden::GardenConfig::default(),
            auto_capture: AutoCaptureConfig::default(),
            auto_extraction: AutoExtractionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoCaptureConfig {
    pub enabled: bool,
}

impl Default for AutoCaptureConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoExtractionConfig {
    pub enabled: bool,
    pub every_n_turns: u32,
}

impl Default for AutoExtractionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            every_n_turns: 5,
        }
    }
}
