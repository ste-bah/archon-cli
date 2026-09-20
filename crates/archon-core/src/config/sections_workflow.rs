//! `[workflow]` and `[workflow.generated]`.
//!
//! Split from `sections.rs` to hold the 500-line ceiling.

use archon_tools::workflow_read_guard::{TreeWideMutator, default_tree_wide_mutators};
use serde::{Deserialize, Serialize};
#[path = "sections_acceptance_execution.rs"]
mod acceptance_execution;
pub use acceptance_execution::*;
#[path = "sections_repository_audit.rs"]
mod repository_audit;
pub use repository_audit::*;

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
    pub repository_audit: RepositoryAuditConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_execution: Option<AcceptanceExecutionConfig>,
    pub generated: GeneratedWorkflowConfig,

    /// Correctness-gate disposition, read once when the process starts.
    pub gate_mode: GateMode,

    /// The code repository `workflow decompose` grounds its authors in.
    /// `[workflow] repository_root = "<PATH>"`.
    ///
    /// The decomposition's authors and critics verify source paths, test
    /// names and module layout here and nowhere else (Issue-55). When the
    /// project directory is not the repository — a project of PRDs and task
    /// sets beside the code they describe — this is what points the authors
    /// at the code. `--repository <PATH>` on the command overrides it;
    /// `[workflow.acceptance_execution].repository`, when configured, is the
    /// fallback; with none of the three the launch refuses rather than guess
    /// the working directory. Relative paths resolve against the working
    /// directory; the path must be an existing git checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_root: Option<std::path::PathBuf>,

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
    /// Inspection calls allowed until a successful substantive file write.
    pub max_reads_before_first_write: u32,
    /// Reads granted after each substantive write before reads are refused again.
    pub reads_per_write: u32,
    /// Permit release Cargo builds for write-capable workflow agents only.
    pub allow_release_builds: bool,
    /// Permit history/worktree-mutating git in write-capable workflow agents. The write coordinator owns git; default false.
    pub allow_git_mutation: bool,
    /// Permit formatters and fixers that rewrite the whole tree without an
    /// explicit scope (`cargo fmt --all`, `black .`, `prettier --write .`) in
    /// write-capable workflow agents. Off by default: every file such a run
    /// touches outside the agent's declared targets is an undeclared change
    /// the patch has to drop (Issue-13). The scoped forms are always allowed.
    pub allow_tree_wide_mutators: bool,
    /// The command shapes refused as tree-wide mutators unless scoped. Unset
    /// means the built-in list (`archon_tools::workflow_read_guard::default_tree_wide_mutators`);
    /// a list here REPLACES it. Inert when `allow_tree_wide_mutators` is on.
    #[serde(skip_serializing_if = "is_default_tree_wide_mutators")]
    pub tree_wide_mutators: Vec<TreeWideMutator>,
    /// Inspection calls (Read, Grep, Glob, read-only shell commands) a
    /// READ-ONLY workflow call may make before every further inspection
    /// result carries a one-line nudge to produce the deliverable (Issue-58).
    /// A read-only call's deliverable is its final message, so no write can
    /// earn it more reading; the nudge is how it is told to stop. `0` turns
    /// the nudge off.
    pub read_only_soft_call_ceiling: u32,
    /// Inspection calls a read-only workflow call may make before further
    /// inspection is refused with the instruction to answer from what it has
    /// read. Build and test commands are not inspection and still run; the
    /// session is not ended. `0` turns the refusal off. Live, an author made
    /// 129 Read/Grep/Glob calls over 80 minutes with nothing to show, bounded
    /// only by the host call timeout.
    pub read_only_hard_call_ceiling: u32,
    pub max_repair_iterations: u8,
    pub max_investigation_iterations: u8,
    pub verification_branch_timeout_secs: u32,
    pub host_call_timeout_secs: u32,
    /// Total wall clock one write branch may spend across its re-dispatches.
    ///
    /// `0` — the default — keeps the derived bound of three host call timeouts.
    /// Partial work survives the cut (it is captured and resumed by the next
    /// attempt at the task), so a shorter budget costs nothing but turns a
    /// six-hour silence into a visible checkpoint. Override per project with
    /// workflow.generated.write_call_time_budget_secs.
    pub write_call_time_budget_secs: u32,
    /// Tool calls a write agent may still make after the host has told it
    /// every declared focused test passed, before inspection and build/test
    /// calls are refused. Write and Edit are never refused. Inert for a task
    /// that declares no focused tests.
    pub submit_grace_calls: u32,
    /// Wall clock for the single in-run retry of a write branch that timed out
    /// with partial work captured. The retry is dispatched with that work
    /// applied and told the declared tests are believed to pass; the smaller of
    /// this and `host_call_timeout_secs` bounds it.
    pub timeout_retry_budget_secs: u32,
    /// How many of the previous session's most recent tool calls a resumed,
    /// retried or restarted write session is shown, beside every call the
    /// host refused it. `0` shows the refusals alone; capped at 50.
    pub resume_memory_calls: u32,
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

fn is_default_tree_wide_mutators(rules: &[TreeWideMutator]) -> bool {
    *rules == default_tree_wide_mutators()
}

impl Default for GeneratedWorkflowConfig {
    fn default() -> Self {
        Self {
            max_reads_before_first_write: 40,
            reads_per_write: 20,
            allow_release_builds: false,
            allow_git_mutation: false,
            allow_tree_wide_mutators: false,
            tree_wide_mutators: default_tree_wide_mutators(),
            read_only_soft_call_ceiling: 80,
            read_only_hard_call_ceiling: 120,
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
            write_call_time_budget_secs: 0,
            submit_grace_calls: 15,
            timeout_retry_budget_secs: 1_800,
            resume_memory_calls: 12,
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
