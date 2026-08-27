//! `[workflow]` and `[workflow.generated]`.
//!
//! Split from `sections.rs` to hold the 500-line ceiling.

use serde::{Deserialize, Serialize};

/// Disposition of decomposition-time correctness gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    Off,
    Observe,
    Enforce,
}

impl Default for GateMode {
    fn default() -> Self {
        Self::Observe
    }
}

/// Workflow runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WorkflowRuntimeConfig {
    pub generated: GeneratedWorkflowConfig,

    /// Correctness-gate disposition, read once when the process starts.
    pub gate_mode: GateMode,

    /// Refuse a workflow agent's writes outside the directories its run
    /// declared. `[workflow] write_confinement = true`.
    ///
    /// Off by default, and the default is a statement about *blast radius*
    /// rather than about whether confinement is a good idea. Turning this on
    /// changes what a long unattended run is permitted to do, and the failure
    /// mode of getting the root set slightly wrong is a stage that cannot write
    /// its own deliverable — hours in, with the refusal buried in an agent
    /// transcript. That is a worse day than the leak it prevents, so an
    /// operator opts into it per project once the declared artifact roots for
    /// that project are known to be right.
    ///
    /// It applies to WORKFLOW agents only. Interactive subagents are untouched
    /// no matter what this says: a user who adds a directory with `/add-dir`
    /// added it because they intend to edit in it, and a session-wide policy
    /// silently demoting those to read-only would be a control nobody asked
    /// for. The scoping is enforced where the roots are attached, not here —
    /// see `SubagentPipelineClient::declared_write_roots`.
    ///
    /// Enabling it confines nothing on a run whose host declared no artifact
    /// roots. That is deliberate and is logged rather than guessed at: the
    /// alternative — falling back to "the working directory" — is what an
    /// earlier attempt did, and it refuses exactly the writes a task exists to
    /// make whenever the deliverable lives outside the tree the agent runs in.
    pub write_confinement: bool,
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
    /// How many isolated agents may hold a reusable build-cache directory at
    /// once.
    ///
    /// Each slot is a directory that outlives the agent using it, so the next
    /// occupant builds incrementally rather than from nothing. The count is
    /// what bounds disk — slots × one cache, however many tasks a run has —
    /// so raising it trades disk for concurrency and lowering it trades the
    /// other way.
    ///
    /// `None` means "as many as may build concurrently", which is the fan-out
    /// width when one is set and otherwise a single slot: a run whose agents
    /// never overlap needs exactly one directory, and one warm directory is
    /// the fastest arrangement there is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_cache_slots: Option<u8>,
}

impl Default for GeneratedWorkflowConfig {
    fn default() -> Self {
        Self {
            max_repair_iterations: 6,
            max_investigation_iterations: 6,
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
            build_cache_slots: None,
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
