//! Pipeline runner loop.
//!
//! Provides the [`PipelineFacade`] trait and [`run_pipeline`] function that
//! implement a shared, context-isolated agent execution loop used by all
//! pipeline types (coding, research, learning, knowledge-base).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::audit::runtime::{PipelineAuditRun, PromptHashes};
use crate::audit::types::PipelineEvent;
use crate::learning::integration::LearningIntegration;
use crate::learning::reflexion::ReflexionInjector;
use crate::research::final_artifact::write_final_research_artifacts;

mod quality_gate;
mod single_agent;
mod support;
mod wave;

pub use quality_gate::PipelineRunOptions;
#[cfg(test)]
use quality_gate::attempt_accepted;
use single_agent::run_single_agent;
pub use support::PipelineProgressFacade;
use wave::run_parallel_wave;

const PIPELINE_MAX_ATTEMPTS: usize = 3;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The kind of pipeline being executed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PipelineType {
    Coding,
    Research,
    Learning,
    Kb,
    GameTheory,
    Workflow,
}

/// Determines what tools an agent is allowed to invoke.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolAccessLevel {
    ReadOnly,
    Full,
}

/// Metadata describing a single agent in the pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub key: String,
    pub display_name: String,
    pub model: String,
    pub phase: u32,
    pub critical: bool,
    #[serde(default)]
    pub parallelizable: bool,
    pub quality_threshold: f64,
    pub tool_access_level: ToolAccessLevel,
}

/// A quality assessment produced by the facade after an agent completes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QualityScore {
    pub overall: f64,
    pub dimensions: HashMap<String, f64>,
}

/// A single tool-use event recorded during agent execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolUseEntry {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
}

/// The outcome of running one agent through the LLM.
#[derive(Clone, Debug)]
pub struct AgentResult {
    pub output: String,
    pub tool_use_log: Vec<ToolUseEntry>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub duration: Duration,
    pub quality: Option<QualityScore>,
}

/// Instruction from the facade about what to do next.
pub enum NextAgent {
    /// Execute this agent next.
    Continue(AgentInfo),
    /// Execute these independent agents as one deterministic bounded wave.
    ContinueWave(Vec<AgentInfo>),
    /// Pipeline is finished.
    Done,
    /// Skip an agent, with a reason string for logging.
    Skip(String),
}

/// Mutable session state threaded through the pipeline run.
pub struct PipelineSession {
    pub id: String,
    pub pipeline_type: PipelineType,
    pub task: String,
    pub started_at: Instant,
    pub agent_results: Vec<(AgentInfo, AgentResult)>,
    /// LEANN code-context injected before each agent prompt build.
    /// Empty string when LEANN is not configured.
    pub leann_context: String,
}

/// The final output of a completed pipeline run.
pub struct PipelineResult {
    pub session_id: String,
    pub pipeline_type: PipelineType,
    pub agent_results: Vec<(AgentInfo, AgentResult)>,
    pub total_cost_usd: f64,
    pub duration: Duration,
    pub final_output: String,
}

// ---------------------------------------------------------------------------
// LEANN integration
// ---------------------------------------------------------------------------

/// Format a set of LEANN search results as markdown code blocks suitable for
/// inclusion in an agent prompt.
///
/// Returns an empty string when `results` is empty so callers can simply
/// concatenate without checking.
pub fn format_leann_results(results: &[archon_leann::SearchResult]) -> String {
    if results.is_empty() {
        return String::new();
    }

    let mut out = String::from("## Code Context\n");
    for r in results {
        out.push_str(&format!(
            "\n### `{}` (lines {}-{})\n```{}\n{}\n```\n",
            r.file_path.display(),
            r.line_start,
            r.line_end,
            r.language,
            r.content,
        ));
    }
    out
}

/// Scan a tool-use log for Write and Edit tool entries and extract the
/// `file_path` values from their `input` JSON.
///
/// Duplicate paths are deduplicated. Entries with missing or non-string
/// `file_path` keys are silently skipped.
pub fn extract_modified_files(tool_use_log: &[ToolUseEntry]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();

    for entry in tool_use_log {
        match entry.tool_name.as_str() {
            "Write" | "Edit" => {
                if let Some(fp) = entry.input.get("file_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(fp);
                    if seen.insert(p.clone()) {
                        paths.push(p);
                    }
                }
            }
            _ => {}
        }
    }

    paths
}

/// Wraps LEANN operations for pipeline integration.
///
/// All operations are resilient: failures are logged as warnings but never
/// propagate errors that would abort the pipeline.
pub struct LeannIntegration {
    code_index: Arc<archon_leann::CodeIndex>,
}

impl LeannIntegration {
    /// Create a new integration wrapper around an existing [`CodeIndex`].
    pub fn new(code_index: Arc<archon_leann::CodeIndex>) -> Self {
        Self { code_index }
    }

    /// Expose the inner [`CodeIndex`] so callers can build
    /// [`LeannSearcher`](crate::leann_searcher::LeannSearcher)
    /// implementations (e.g. for the research pipeline facade).
    pub fn code_index(&self) -> &Arc<archon_leann::CodeIndex> {
        &self.code_index
    }

    /// Index the repository on pipeline startup.
    ///
    /// Logs a warning and returns `Ok(())` if indexing fails so the pipeline
    /// can proceed without LEANN.
    pub async fn init_repository(&self, working_dir: &std::path::Path) -> Result<()> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        self.init_repository_blocking_with_cancel(working_dir, &cancel)
    }

    /// Blocking repository indexing entrypoint with cooperative cancellation.
    ///
    /// Session startup runs this through `spawn_blocking` so tree-sitter,
    /// ONNX embedding, and Cozo writes never occupy Tokio worker threads. The
    /// cancellation flag is checked between files and batches inside LEANN.
    pub fn init_repository_blocking_with_cancel(
        &self,
        working_dir: &std::path::Path,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        self.init_repository_blocking_with_excludes(working_dir, &[], cancel)
    }

    /// As above, plus the project's own excluded directory names.
    ///
    /// `[code_index] exclude_patterns` reaches the walk through here. It did
    /// not before: the list below was a `vec![...]` literal and the config key
    /// did not exist, so a project could not tell the indexer to skip anything.
    pub fn init_repository_blocking_with_excludes(
        &self,
        working_dir: &std::path::Path,
        extra_excludes: &[String],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        let config = archon_leann::IndexConfig {
            root_path: working_dir.to_path_buf(),
            include_patterns: vec!["**/*.rs".into(), "**/*.py".into(), "**/*.ts".into()],
            // Directory NAMES, not globs. `is_excluded` compares path
            // components, so `**/target/**` matched nothing and this indexed
            // `target/`, `node_modules/` and `.git/` in full. These three are
            // already in `default_exclude_patterns`, which now always applies,
            // so this list is belt-and-braces rather than the only guard.
            exclude_patterns: {
                let mut excludes: Vec<String> =
                    vec!["target".into(), "node_modules".into(), ".git".into()];
                excludes.extend(extra_excludes.iter().cloned());
                excludes
            },
        };
        match self
            .code_index
            .index_repository_blocking_with_cancel(working_dir, &config, cancel)
        {
            Ok(stats) => {
                tracing::info!(
                    files = stats.total_files,
                    chunks = stats.total_chunks,
                    "LEANN repository indexed"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "LEANN repository indexing failed; continuing without index");
            }
        }
        Ok(())
    }

    /// Search for code context relevant to the current agent.
    ///
    /// Combines `task` and `agent_key` into a query, searches with limit 5,
    /// and formats the results as markdown. Returns an empty string on any
    /// failure.
    pub fn search_context(&self, task: &str, agent_key: &str) -> String {
        let query = format!("{} {}", task, agent_key);
        match self.code_index.search_code(&query, 5) {
            Ok(results) => format_leann_results(&results),
            Err(e) => {
                tracing::warn!(error = %e, "LEANN search failed; using empty context");
                String::new()
            }
        }
    }

    /// Index files modified by an agent (intended for Phase 4+ agents).
    ///
    /// Returns the number of files successfully indexed. Failures on
    /// individual files are logged but do not abort the operation.
    pub async fn index_modified_files(&self, tool_use_log: &[ToolUseEntry]) -> Result<usize> {
        let paths = extract_modified_files(tool_use_log);
        let mut indexed = 0usize;
        for path in &paths {
            match self.code_index.index_file(path).await {
                Ok(()) => {
                    indexed += 1;
                    tracing::debug!(path = %path.display(), "LEANN indexed modified file");
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "LEANN failed to index modified file; continuing"
                    );
                }
            }
        }
        Ok(indexed)
    }
}

// ---------------------------------------------------------------------------
// LLM Client trait
// ---------------------------------------------------------------------------

/// Response returned by an [`LlmClient`] implementation.
#[derive(Clone, Debug)]
pub struct LlmResponse {
    pub content: String,
    pub tool_uses: Vec<ToolUseEntry>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Provider completion reason. `None` only when the underlying execution
    /// surface cannot expose one (for example a completed full subagent loop).
    pub stop_reason: Option<String>,
}

/// Full context needed to execute one pipeline agent.
///
/// Implementations may run this as a plain provider completion or as a real
/// tool-capable subagent. The default [`LlmClient`] implementation preserves
/// the legacy provider-completion path.
#[derive(Clone, Debug)]
pub struct AgentExecutionRequest {
    pub session_id: String,
    pub pipeline_type: PipelineType,
    pub task: String,
    pub cwd: Option<PathBuf>,
    pub ordinal: usize,
    pub attempt: usize,
    pub agent: AgentInfo,
    pub messages: Vec<serde_json::Value>,
    pub system: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub allowed_tools: Vec<String>,
    pub timeout_secs: Option<u64>,
    pub disable_auto_background: bool,
    /// Absolute directories the caller declares this agent may write.
    ///
    /// Only the workflow path sets it, and only the workflow path is confined
    /// by it — see `SubagentPipelineClient::declared_write_roots`. Every other
    /// pipeline leaves it empty and is unaffected, which is deliberate: an
    /// interactive subagent's directories were chosen by a user who intends to
    /// edit in them.
    pub write_roots: Vec<String>,
    pub provider_env_resolution: Option<archon_tools::provider_env::ProviderEnvResolution>,
}

/// Abstraction over the underlying LLM transport. Concrete implementations
/// live in `archon-llm`; the pipeline crate depends only on this trait.
#[async_trait]
pub trait LlmClient: Send + Sync {
    fn provider_id(&self) -> Option<String> {
        None
    }

    fn resolve_model_alias(&self, model: &str) -> String {
        model.to_string()
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> Result<LlmResponse>;

    async fn send_message_with_temperature(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> Result<LlmResponse> {
        anyhow::bail!("client does not support explicit sampling")
    }

    /// Continue a completed invocation rather than creating a fresh agent.
    async fn continue_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.run_agent(request).await
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        let model = request.agent.model.clone();
        self.send_message(request.messages, request.system, request.tools, &model)
            .await
    }
}

// ---------------------------------------------------------------------------
// Pipeline Facade trait
// ---------------------------------------------------------------------------

/// Domain-specific behaviour injected into the shared runner loop.
///
/// Each pipeline type (coding, research, ...) implements this trait to control
/// agent ordering, prompt construction, quality scoring, and finalization.
#[async_trait]
pub trait PipelineFacade: Send + Sync {
    /// Create a fresh session for the given task description.
    async fn init_session(&self, task: &str) -> Result<PipelineSession>;

    /// Determine the next agent to run (or signal completion / skip).
    async fn next_agent(&self, session: &PipelineSession) -> Result<NextAgent>;

    /// Build the (messages, system, tools) triple for the given agent.
    ///
    /// Each call should return a **fresh** set of messages to ensure context
    /// isolation between agents.
    async fn build_prompt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )>;

    async fn build_prompt_for_attempt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
        _attempt: u8,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        self.build_prompt(session, agent).await
    }

    /// Score the quality of an agent's output after execution.
    async fn score_quality(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
    ) -> Result<QualityScore>;

    /// Post-processing hook called after each agent completes (e.g. persist
    /// artifacts, update session metadata).
    async fn process_completion(
        &self,
        session: &mut PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
        quality: &QualityScore,
    ) -> Result<()>;

    /// Produce the final [`PipelineResult`] once all agents have finished.
    async fn finalize(&self, session: PipelineSession) -> Result<PipelineResult>;
}

mod execution;
use execution::{
    fail_audit, is_context_window_error, is_retryable_pipeline_attempt_error,
    pipeline_attempt_retry_delay, quality_gate_failure, relative_to_bundle,
};
pub use execution::{
    resume_pipeline_audited, resume_pipeline_audited_with_options, run_pipeline,
    run_pipeline_audited,
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
