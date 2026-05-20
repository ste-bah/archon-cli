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

use futures_util::future::join_all;

use crate::audit::runtime::{PipelineAuditRun, PromptHashes};
use crate::audit::types::PipelineEvent;
use crate::learning::integration::LearningIntegration;
use crate::learning::reflexion::{FailedTrajectory, ReflexionInjector};
use crate::research::artifacts::write_research_agent_artifacts;
use crate::research::final_artifact::write_final_research_artifacts;

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
        let config = archon_leann::IndexConfig {
            root_path: working_dir.to_path_buf(),
            include_patterns: vec!["**/*.rs".into(), "**/*.py".into(), "**/*.ts".into()],
            exclude_patterns: vec![
                "**/target/**".into(),
                "**/node_modules/**".into(),
                "**/.git/**".into(),
            ],
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
    pub ordinal: usize,
    pub attempt: usize,
    pub agent: AgentInfo,
    pub messages: Vec<serde_json::Value>,
    pub system: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub allowed_tools: Vec<String>,
}

/// Abstraction over the underlying LLM transport. Concrete implementations
/// live in `archon-llm`; the pipeline crate depends only on this trait.
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> Result<LlmResponse>;

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

// ---------------------------------------------------------------------------
// Runner loop
// ---------------------------------------------------------------------------

/// Execute a full pipeline run.
///
/// The runner repeatedly asks the facade for the next agent, builds a fresh
/// prompt (context isolation), sends it to the LLM, scores quality, and
/// records the result. Once the facade signals [`NextAgent::Done`], it
/// finalizes and returns the [`PipelineResult`].
pub async fn run_pipeline(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    task: &str,
    leann: Option<&LeannIntegration>,
    mut reflexion: Option<&mut ReflexionInjector>,
    mut learning: Option<&mut LearningIntegration>,
) -> Result<PipelineResult> {
    let mut session = facade.init_session(task).await?;
    run_pipeline_inner(
        facade,
        llm,
        &mut session,
        leann,
        &mut reflexion,
        &mut learning,
        None,
    )
    .await
}

/// Execute a built-in pipeline with a durable audited bundle.
pub async fn run_pipeline_audited(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    task: &str,
    worktree: &Path,
    leann: Option<&LeannIntegration>,
    mut reflexion: Option<&mut ReflexionInjector>,
    mut learning: Option<&mut LearningIntegration>,
) -> Result<PipelineResult> {
    let mut session = facade.init_session(task).await?;
    let pipeline_type = session.pipeline_type.clone();
    let audit = PipelineAuditRun::start(worktree, &session.id, pipeline_type, &session.task)?;
    run_pipeline_inner(
        facade,
        llm,
        &mut session,
        leann,
        &mut reflexion,
        &mut learning,
        Some(audit),
    )
    .await
}

/// Resume a verified audited bundle without repeating completed agents.
pub async fn resume_pipeline_audited(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session_id: &str,
    worktree: &Path,
    leann: Option<&LeannIntegration>,
    mut reflexion: Option<&mut ReflexionInjector>,
    mut learning: Option<&mut LearningIntegration>,
) -> Result<PipelineResult> {
    let audit = PipelineAuditRun::resume(worktree, session_id)?;
    let mut session = facade.init_session(&audit.state().task).await?;
    session.id = audit.state().session_id.clone();
    session.pipeline_type = audit.state().pipeline_type.clone();
    session.agent_results = audit.hydrate_results()?;
    run_pipeline_inner(
        facade,
        llm,
        &mut session,
        leann,
        &mut reflexion,
        &mut learning,
        Some(audit),
    )
    .await
}

async fn run_pipeline_inner(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session: &mut PipelineSession,
    leann: Option<&LeannIntegration>,
    reflexion: &mut Option<&mut ReflexionInjector>,
    learning: &mut Option<&mut LearningIntegration>,
    mut audit: Option<PipelineAuditRun>,
) -> Result<PipelineResult> {
    tracing::info!(
        session_id = %session.id,
        pipeline_type = ?session.pipeline_type,
        task = %session.task,
        leann_enabled = leann.is_some(),
        "Pipeline session initialised"
    );

    loop {
        let next = match facade.next_agent(session).await {
            Ok(next) => next,
            Err(error) => {
                fail_audit(&mut audit, &error.to_string())?;
                return Err(error);
            }
        };
        match next {
            NextAgent::Continue(agent) => {
                let ordinal = session.agent_results.len();
                tracing::info!(
                    agent_key = %agent.key,
                    agent_name = %agent.display_name,
                    phase = agent.phase,
                    model = %agent.model,
                    "Executing agent"
                );
                if let Some(audit) = audit.as_mut() {
                    audit.record_agent_planned(ordinal, &agent)?;
                }

                // Inject LEANN code context before prompt build.
                if let Some(li) = leann {
                    session.leann_context = li.search_context(&session.task, &agent.key);
                }

                // v0.1.23: Query learning integration for SONA + ReasoningBank context.
                let learning_ctx = if let Some(li) = learning.as_deref_mut() {
                    li.on_agent_start(
                        &agent.key,
                        &agent.phase.to_string(),
                        &session.task,
                        &session.id,
                    )
                } else {
                    Default::default()
                };

                // v0.1.23: Retry loop with reflexion injection.
                let mut attempt = 0usize;
                let mut attempts = Vec::new();
                let (result, quality, accepted_prompt) = loop {
                    attempt += 1;

                    // Build a fresh prompt — context isolation.
                    let (messages, mut system, tools) = match facade
                        .build_prompt_for_attempt(session, &agent, attempt as u8)
                        .await
                    {
                        Ok(prompt) => prompt,
                        Err(error) => {
                            fail_audit(&mut audit, &error.to_string())?;
                            return Err(error);
                        }
                    };

                    // Inject learning context on first attempt.
                    if attempt == 1 {
                        if !learning_ctx.sona_context.is_empty() {
                            system.push(serde_json::json!({
                                "text": format!("## SONA Trajectory\n{}", learning_ctx.sona_context),
                            }));
                        }
                        if !learning_ctx.reasoning_context.is_empty() {
                            system.push(serde_json::json!({
                                "text": format!("## Reasoning Context\n{}", learning_ctx.reasoning_context),
                            }));
                        }
                        if !learning_ctx.desc_episodes.is_empty() {
                            system.push(serde_json::json!({
                                "text": format!(
                                    "## DESC Episodes\n{}",
                                    learning_ctx.desc_episodes.join("\n\n")
                                ),
                            }));
                        }
                    }

                    // Inject reflexion context on retry.
                    if attempt > 1
                        && let Some(ri) = reflexion.as_deref()
                        && let Some(ctx) = ri.inject_reflexion(&agent.key)
                    {
                        system.push(serde_json::json!({
                            "text": ctx.formatted_prompt_section,
                        }));
                        tracing::info!(
                            agent_key = %agent.key,
                            attempt = attempt,
                            "Reflexion context injected"
                        );
                    }

                    let prompt_hashes = if let Some(audit) = audit.as_ref() {
                        Some(audit.record_prompt(ordinal, &agent, &messages, &system, &tools)?)
                    } else {
                        None
                    };
                    if let Some(audit) = audit.as_ref() {
                        audit.record_attempt_started(ordinal, &agent, attempt)?;
                    }

                    // Execute against the LLM.
                    let agent_start = Instant::now();
                    let llm_response = match llm
                        .run_agent(AgentExecutionRequest {
                            session_id: session.id.clone(),
                            pipeline_type: session.pipeline_type.clone(),
                            task: session.task.clone(),
                            ordinal,
                            attempt,
                            agent: agent.clone(),
                            messages,
                            system,
                            tools,
                            allowed_tools: Vec::new(),
                        })
                        .await
                    {
                        Ok(response) => response,
                        Err(error) => {
                            if is_context_window_error(&error) && attempt < PIPELINE_MAX_ATTEMPTS {
                                tracing::warn!(
                                    agent_key = %agent.key,
                                    "pipeline prompt exceeded context; rebuilding with retry budget"
                                );
                                continue;
                            }
                            if is_retryable_pipeline_attempt_error(&error)
                                && attempt < PIPELINE_MAX_ATTEMPTS
                            {
                                let reason = error.to_string();
                                if let Some(audit) = audit.as_ref() {
                                    audit
                                        .record_attempt_failed(ordinal, &agent, attempt, &reason)?;
                                    audit.record_agent_retry(ordinal, &agent, attempt, &reason)?;
                                }
                                let delay = pipeline_attempt_retry_delay(attempt);
                                tracing::warn!(
                                    agent_key = %agent.key,
                                    attempt = attempt,
                                    retry_delay_ms = delay.as_millis() as u64,
                                    error = %reason,
                                    "pipeline agent attempt failed with retryable transport error"
                                );
                                tokio::time::sleep(delay).await;
                                continue;
                            }
                            if let Some(audit) = audit.as_ref() {
                                audit.record_attempt_failed(
                                    ordinal,
                                    &agent,
                                    attempt,
                                    &error.to_string(),
                                )?;
                            }
                            fail_audit(&mut audit, &error.to_string())?;
                            return Err(error);
                        }
                    };
                    let duration = agent_start.elapsed();

                    // Build the agent result.
                    let mut result = AgentResult {
                        output: llm_response.content,
                        tool_use_log: llm_response.tool_uses,
                        tokens_in: llm_response.tokens_in,
                        tokens_out: llm_response.tokens_out,
                        cost_usd: 0.0,
                        duration,
                        quality: None,
                    };

                    // Score quality.
                    let quality = match facade.score_quality(session, &agent, &result).await {
                        Ok(quality) => quality,
                        Err(error) => {
                            fail_audit(&mut audit, &error.to_string())?;
                            return Err(error);
                        }
                    };
                    result.quality = Some(quality.clone());

                    let meets_threshold = quality.overall >= agent.quality_threshold;
                    let accepted = meets_threshold || attempt >= PIPELINE_MAX_ATTEMPTS;
                    let failure_reason = (!meets_threshold).then(|| {
                        format!(
                            "Quality {:.2} below threshold {:.2}",
                            quality.overall, agent.quality_threshold
                        )
                    });
                    if let Some(audit) = audit.as_ref() {
                        let attempt_record = audit.record_attempt_completed(
                            ordinal,
                            &agent,
                            attempt,
                            &result,
                            accepted,
                            failure_reason.clone(),
                        )?;
                        attempts.push(attempt_record);
                    }

                    tracing::info!(
                        agent_key = %agent.key,
                        attempt = attempt,
                        quality_overall = quality.overall,
                        threshold = agent.quality_threshold,
                        meets_threshold = meets_threshold,
                        tokens_in = result.tokens_in,
                        tokens_out = result.tokens_out,
                        duration_ms = duration.as_millis() as u64,
                        "Agent completed"
                    );

                    if accepted {
                        break (result, quality, prompt_hashes);
                    }

                    // Record failure for reflexion on next attempt.
                    if let Some(ri) = reflexion.as_deref_mut() {
                        ri.record_failure(FailedTrajectory {
                            agent_name: agent.key.clone(),
                            attempt,
                            output_summary: result.output.clone(),
                            failure_reason: format!(
                                "Quality {:.2} below threshold {:.2}",
                                quality.overall, agent.quality_threshold
                            ),
                            quality_score: quality.overall,
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        });
                        tracing::info!(
                            agent_key = %agent.key,
                            attempt = attempt,
                            "Recorded failure for reflexion"
                        );
                    }
                    if let Some(audit) = audit.as_ref() {
                        audit.record_agent_retry(
                            ordinal,
                            &agent,
                            attempt,
                            failure_reason
                                .as_deref()
                                .unwrap_or("quality threshold not met"),
                        )?;
                    }
                };

                // Post-processing.
                if let Err(error) = facade
                    .process_completion(session, &agent, &result, &quality)
                    .await
                {
                    fail_audit(&mut audit, &error.to_string())?;
                    return Err(error);
                }

                // v0.1.23: Feed quality back into learning integration (SONA trajectory).
                if let Some(li) = learning.as_deref_mut() {
                    li.on_agent_complete(&agent.key, quality.overall, &result.output);
                }

                // Re-index modified files for implementation agents (Phase 4+).
                if agent.phase >= 4
                    && let Some(li) = leann
                {
                    match li.index_modified_files(&result.tool_use_log).await {
                        Ok(count) if count > 0 => {
                            tracing::info!(
                                agent_key = %agent.key,
                                files_indexed = count,
                                "LEANN re-indexed modified files"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                agent_key = %agent.key,
                                error = %e,
                                "LEANN re-indexing failed; continuing"
                            );
                        }
                        _ => {}
                    }
                }

                // Store in session.
                if let Some(audit) = audit.as_mut() {
                    if let Some(prompt) = accepted_prompt {
                        audit.record_agent_completed(ordinal, &agent, &result, attempts, prompt)?;
                        if session.pipeline_type == PipelineType::Research {
                            let bundle_dir = audit.store().bundle_dir(&session.id);
                            for artifact in write_research_agent_artifacts(
                                &bundle_dir,
                                ordinal,
                                &agent.key,
                                &result.output,
                            )? {
                                audit.store().append_event(
                                    &session.id,
                                    PipelineEvent::ArtifactWritten {
                                        artifact_type: "research-agent-output".to_string(),
                                        path: relative_to_bundle(&bundle_dir, &artifact.path),
                                        content_hash: artifact.hash,
                                    },
                                )?;
                            }
                        }
                    }
                }
                session.agent_results.push((agent, result));
            }
            NextAgent::ContinueWave(agents) => {
                let agents: Vec<AgentInfo> = agents
                    .into_iter()
                    .take(4)
                    .filter(|agent| agent.parallelizable)
                    .collect();
                if agents.is_empty() {
                    continue;
                }
                let ordinal_start = session.agent_results.len();
                tracing::info!(
                    session_id = %session.id,
                    wave_size = agents.len(),
                    agent_keys = %agents.iter().map(|agent| agent.key.as_str()).collect::<Vec<_>>().join(","),
                    "pipeline.agent.parallel_wave_started"
                );

                struct PreparedWaveAgent {
                    ordinal: usize,
                    agent: AgentInfo,
                    session: PipelineSession,
                    messages: Vec<serde_json::Value>,
                    system: Vec<serde_json::Value>,
                    tools: Vec<serde_json::Value>,
                    prompt: Option<PromptHashes>,
                }

                async fn run_wave_attempt(
                    facade: &dyn PipelineFacade,
                    llm: &dyn LlmClient,
                    audit: &Option<PipelineAuditRun>,
                    prepared: &PreparedWaveAgent,
                    attempt: usize,
                    reflexion_section: Option<String>,
                ) -> Result<(AgentResult, QualityScore, Option<PromptHashes>)> {
                    let (messages, mut system, tools) = facade
                        .build_prompt_for_attempt(&prepared.session, &prepared.agent, attempt as u8)
                        .await?;
                    if let Some(section) = reflexion_section {
                        system.push(serde_json::json!({
                            "text": section,
                        }));
                        tracing::info!(
                            agent_key = %prepared.agent.key,
                            attempt = attempt,
                            "Reflexion context injected"
                        );
                    }
                    let prompt = if let Some(audit) = audit.as_ref() {
                        Some(audit.record_prompt(
                            prepared.ordinal,
                            &prepared.agent,
                            &messages,
                            &system,
                            &tools,
                        )?)
                    } else {
                        None
                    };
                    if let Some(audit) = audit.as_ref() {
                        audit.record_attempt_started(prepared.ordinal, &prepared.agent, attempt)?;
                    }
                    let start = Instant::now();
                    let response = llm
                        .run_agent(AgentExecutionRequest {
                            session_id: prepared.session.id.clone(),
                            pipeline_type: prepared.session.pipeline_type.clone(),
                            task: prepared.session.task.clone(),
                            ordinal: prepared.ordinal,
                            attempt,
                            agent: prepared.agent.clone(),
                            messages,
                            system,
                            tools,
                            allowed_tools: Vec::new(),
                        })
                        .await?;
                    let mut result = AgentResult {
                        output: response.content,
                        tool_use_log: response.tool_uses,
                        tokens_in: response.tokens_in,
                        tokens_out: response.tokens_out,
                        cost_usd: 0.0,
                        duration: start.elapsed(),
                        quality: None,
                    };
                    let quality = facade
                        .score_quality(&prepared.session, &prepared.agent, &result)
                        .await?;
                    result.quality = Some(quality.clone());
                    Ok((result, quality, prompt))
                }

                let mut prepared = Vec::with_capacity(agents.len());
                for (offset, agent) in agents.into_iter().enumerate() {
                    let ordinal = ordinal_start + offset;
                    tracing::info!(
                        agent_key = %agent.key,
                        agent_name = %agent.display_name,
                        phase = agent.phase,
                        model = %agent.model,
                        wave_ordinal = ordinal,
                        "Executing agent in parallel wave"
                    );
                    if let Some(audit) = audit.as_mut() {
                        audit.record_agent_planned(ordinal, &agent)?;
                    }

                    let leann_context = leann
                        .map(|li| li.search_context(&session.task, &agent.key))
                        .unwrap_or_default();
                    let mut agent_session = PipelineSession {
                        id: session.id.clone(),
                        pipeline_type: session.pipeline_type.clone(),
                        task: session.task.clone(),
                        started_at: session.started_at,
                        agent_results: session.agent_results.clone(),
                        leann_context,
                    };

                    let learning_ctx = if let Some(li) = learning.as_deref_mut() {
                        li.on_agent_start(
                            &agent.key,
                            &agent.phase.to_string(),
                            &session.task,
                            &session.id,
                        )
                    } else {
                        Default::default()
                    };

                    let (messages, mut system, tools) = facade
                        .build_prompt_for_attempt(&agent_session, &agent, 1)
                        .await?;
                    if !learning_ctx.sona_context.is_empty() {
                        system.push(serde_json::json!({
                            "text": format!("## SONA Trajectory\n{}", learning_ctx.sona_context),
                        }));
                    }
                    if !learning_ctx.reasoning_context.is_empty() {
                        system.push(serde_json::json!({
                            "text": format!("## Reasoning Context\n{}", learning_ctx.reasoning_context),
                        }));
                    }
                    if !learning_ctx.desc_episodes.is_empty() {
                        system.push(serde_json::json!({
                            "text": format!(
                                "## DESC Episodes\n{}",
                                learning_ctx.desc_episodes.join("\n\n")
                            ),
                        }));
                    }

                    let prompt = if let Some(audit) = audit.as_ref() {
                        Some(audit.record_prompt(ordinal, &agent, &messages, &system, &tools)?)
                    } else {
                        None
                    };
                    if let Some(audit) = audit.as_ref() {
                        audit.record_attempt_started(ordinal, &agent, 1)?;
                    }

                    // Keep a per-agent snapshot so prompt/quality reads remain
                    // isolated while completions are later committed in order.
                    agent_session.leann_context = agent_session.leann_context.clone();
                    prepared.push(PreparedWaveAgent {
                        ordinal,
                        agent,
                        session: agent_session,
                        messages,
                        system,
                        tools,
                        prompt,
                    });
                }

                let calls = prepared.into_iter().map(|prepared| async move {
                    let start = Instant::now();
                    let response = match llm
                        .run_agent(AgentExecutionRequest {
                            session_id: prepared.session.id.clone(),
                            pipeline_type: prepared.session.pipeline_type.clone(),
                            task: prepared.session.task.clone(),
                            ordinal: prepared.ordinal,
                            attempt: 1,
                            agent: prepared.agent.clone(),
                            messages: prepared.messages.clone(),
                            system: prepared.system.clone(),
                            tools: prepared.tools.clone(),
                            allowed_tools: Vec::new(),
                        })
                        .await
                    {
                        Ok(response) => response,
                        Err(error) => return Err((prepared, error)),
                    };
                    let mut result = AgentResult {
                        output: response.content,
                        tool_use_log: response.tool_uses,
                        tokens_in: response.tokens_in,
                        tokens_out: response.tokens_out,
                        cost_usd: 0.0,
                        duration: start.elapsed(),
                        quality: None,
                    };
                    let quality = match facade
                        .score_quality(&prepared.session, &prepared.agent, &result)
                        .await
                    {
                        Ok(quality) => quality,
                        Err(error) => return Err((prepared, error)),
                    };
                    result.quality = Some(quality.clone());
                    Ok::<_, (PreparedWaveAgent, anyhow::Error)>((prepared, result, quality))
                });
                let completed = join_all(calls).await;

                for item in completed {
                    let (prepared, mut result, mut quality, mut attempt, mut accepted_prompt) =
                        match item {
                            Ok((prepared, result, quality)) => {
                                let accepted_prompt = prepared.prompt.clone();
                                (prepared, result, quality, 1usize, accepted_prompt)
                            }
                            Err((prepared, error)) => {
                                if !is_context_window_error(&error) {
                                    if let Some(audit) = audit.as_ref() {
                                        audit.record_attempt_failed(
                                            prepared.ordinal,
                                            &prepared.agent,
                                            1,
                                            &error.to_string(),
                                        )?;
                                    }
                                    fail_audit(&mut audit, &error.to_string())?;
                                    return Err(error);
                                }
                                tracing::warn!(
                                    agent_key = %prepared.agent.key,
                                    "pipeline parallel-wave prompt exceeded context; rebuilding with retry budget"
                                );
                                let mut attempt = 1usize;
                                loop {
                                    if attempt >= PIPELINE_MAX_ATTEMPTS {
                                        if let Some(audit) = audit.as_ref() {
                                            audit.record_attempt_failed(
                                                prepared.ordinal,
                                                &prepared.agent,
                                                attempt,
                                                &error.to_string(),
                                            )?;
                                        }
                                        fail_audit(&mut audit, &error.to_string())?;
                                        return Err(error);
                                    }
                                    attempt += 1;
                                    match run_wave_attempt(
                                        facade, llm, &audit, &prepared, attempt, None,
                                    )
                                    .await
                                    {
                                        Ok((result, quality, prompt)) => {
                                            break (prepared, result, quality, attempt, prompt);
                                        }
                                        Err(retry_error)
                                            if is_context_window_error(&retry_error)
                                                && attempt < PIPELINE_MAX_ATTEMPTS =>
                                        {
                                            tracing::warn!(
                                                agent_key = %prepared.agent.key,
                                                attempt = attempt,
                                                "pipeline parallel-wave prompt exceeded context; rebuilding with tighter retry budget"
                                            );
                                            continue;
                                        }
                                        Err(retry_error) => {
                                            if let Some(audit) = audit.as_ref() {
                                                audit.record_attempt_failed(
                                                    prepared.ordinal,
                                                    &prepared.agent,
                                                    attempt,
                                                    &retry_error.to_string(),
                                                )?;
                                            }
                                            fail_audit(&mut audit, &retry_error.to_string())?;
                                            return Err(retry_error);
                                        }
                                    }
                                }
                            }
                        };

                    let mut attempts = Vec::new();
                    loop {
                        let meets_threshold = quality.overall >= prepared.agent.quality_threshold;
                        let failure_reason = (!meets_threshold).then(|| {
                            format!(
                                "Quality {:.2} below threshold {:.2}",
                                quality.overall, prepared.agent.quality_threshold
                            )
                        });
                        let accepted = meets_threshold || attempt >= PIPELINE_MAX_ATTEMPTS;
                        if let Some(audit) = audit.as_ref() {
                            attempts.push(audit.record_attempt_completed(
                                prepared.ordinal,
                                &prepared.agent,
                                attempt,
                                &result,
                                accepted,
                                failure_reason.clone(),
                            )?);
                        }

                        tracing::info!(
                            agent_key = %prepared.agent.key,
                            attempt = attempt,
                            quality_overall = quality.overall,
                            threshold = prepared.agent.quality_threshold,
                            meets_threshold = meets_threshold,
                            tokens_in = result.tokens_in,
                            tokens_out = result.tokens_out,
                            duration_ms = result.duration.as_millis() as u64,
                            "Parallel-wave agent completed"
                        );

                        if accepted {
                            break;
                        }

                        if let Some(ri) = reflexion.as_deref_mut() {
                            ri.record_failure(FailedTrajectory {
                                agent_name: prepared.agent.key.clone(),
                                attempt,
                                output_summary: result.output.clone(),
                                failure_reason: failure_reason
                                    .clone()
                                    .unwrap_or_else(|| "quality threshold not met".to_string()),
                                quality_score: quality.overall,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs(),
                            });
                            tracing::info!(
                                agent_key = %prepared.agent.key,
                                attempt = attempt,
                                "Recorded failure for reflexion"
                            );
                        }
                        if let Some(audit) = audit.as_ref() {
                            audit.record_agent_retry(
                                prepared.ordinal,
                                &prepared.agent,
                                attempt,
                                failure_reason
                                    .as_deref()
                                    .unwrap_or("quality threshold not met"),
                            )?;
                        }

                        attempt += 1;
                        let reflexion_section = reflexion
                            .as_deref()
                            .and_then(|ri| ri.inject_reflexion(&prepared.agent.key))
                            .map(|ctx| ctx.formatted_prompt_section);
                        match run_wave_attempt(
                            facade,
                            llm,
                            &audit,
                            &prepared,
                            attempt,
                            reflexion_section,
                        )
                        .await
                        {
                            Ok((retry_result, retry_quality, retry_prompt)) => {
                                result = retry_result;
                                quality = retry_quality;
                                accepted_prompt = retry_prompt;
                            }
                            Err(error)
                                if is_context_window_error(&error)
                                    && attempt < PIPELINE_MAX_ATTEMPTS =>
                            {
                                tracing::warn!(
                                    agent_key = %prepared.agent.key,
                                    attempt = attempt,
                                    "pipeline parallel-wave prompt exceeded context; rebuilding with tighter retry budget"
                                );
                                continue;
                            }
                            Err(error) => {
                                if let Some(audit) = audit.as_ref() {
                                    audit.record_attempt_failed(
                                        prepared.ordinal,
                                        &prepared.agent,
                                        attempt,
                                        &error.to_string(),
                                    )?;
                                }
                                fail_audit(&mut audit, &error.to_string())?;
                                return Err(error);
                            }
                        }
                    }

                    if let Err(error) = facade
                        .process_completion(session, &prepared.agent, &result, &quality)
                        .await
                    {
                        fail_audit(&mut audit, &error.to_string())?;
                        return Err(error);
                    }
                    if let Some(li) = learning.as_deref_mut() {
                        li.on_agent_complete(&prepared.agent.key, quality.overall, &result.output);
                    }
                    if prepared.agent.phase >= 4
                        && let Some(li) = leann
                    {
                        match li.index_modified_files(&result.tool_use_log).await {
                            Ok(count) if count > 0 => {
                                tracing::info!(
                                    agent_key = %prepared.agent.key,
                                    files_indexed = count,
                                    "LEANN re-indexed modified files"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    agent_key = %prepared.agent.key,
                                    error = %e,
                                    "LEANN re-indexing failed; continuing"
                                );
                            }
                            _ => {}
                        }
                    }
                    if let Some(audit) = audit.as_mut()
                        && let Some(prompt) = accepted_prompt
                    {
                        audit.record_agent_completed(
                            prepared.ordinal,
                            &prepared.agent,
                            &result,
                            attempts,
                            prompt,
                        )?;
                    }
                    session.agent_results.push((prepared.agent, result));
                }
            }
            NextAgent::Skip(reason) => {
                tracing::warn!(reason = %reason, "Skipping agent");
            }
            NextAgent::Done => {
                tracing::info!(
                    session_id = %session.id,
                    agents_executed = session.agent_results.len(),
                    "Pipeline loop complete"
                );
                break;
            }
        }
    }

    let session_id = session.id.clone();
    let pipeline_type = session.pipeline_type.clone();
    let placeholder = PipelineSession {
        id: session_id,
        pipeline_type,
        task: String::new(),
        started_at: Instant::now(),
        agent_results: Vec::new(),
        leann_context: String::new(),
    };
    let owned_session = std::mem::replace(session, placeholder);
    let result = match facade.finalize(owned_session).await {
        Ok(result) => result,
        Err(error) => {
            fail_audit(&mut audit, &error.to_string())?;
            return Err(error);
        }
    };
    if result.pipeline_type == PipelineType::Research
        && let Some(audit_run) = audit.as_ref()
    {
        let bundle_dir = audit_run.store().bundle_dir(&result.session_id);
        match write_final_research_artifacts(&bundle_dir, &result.final_output) {
            Ok(artifacts) => {
                let markdown_path = relative_to_bundle(&bundle_dir, &artifacts.markdown_path);
                let pdf_path = relative_to_bundle(&bundle_dir, &artifacts.pdf_path);
                audit_run.store().append_event(
                    &result.session_id,
                    PipelineEvent::ArtifactWritten {
                        artifact_type: "research-paper-markdown".to_string(),
                        path: markdown_path,
                        content_hash: artifacts.markdown_hash,
                    },
                )?;
                audit_run.store().append_event(
                    &result.session_id,
                    PipelineEvent::ArtifactWritten {
                        artifact_type: "research-paper-pdf".to_string(),
                        path: pdf_path,
                        content_hash: artifacts.pdf_hash,
                    },
                )?;
            }
            Err(error) => {
                let error = error.context("failed to produce final APA research paper artifacts");
                fail_audit(&mut audit, &error.to_string())?;
                return Err(error);
            }
        }
    }
    if let Some(audit) = audit.as_mut() {
        audit.complete(&result.final_output)?;
    }
    Ok(result)
}

fn relative_to_bundle(bundle_dir: &Path, path: &Path) -> String {
    path.strip_prefix(bundle_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn is_context_window_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<archon_llm::provider::LlmError>()
        .is_some_and(|err| err.is_context_window_exceeded())
        || archon_llm::context_window::classify_context_window_error(
            None,
            None,
            None,
            &error.to_string(),
            Some("pipeline"),
            None,
        )
        .is_some()
}

fn is_retryable_pipeline_attempt_error(error: &anyhow::Error) -> bool {
    if let Some(err) = error.downcast_ref::<archon_llm::provider::LlmError>() {
        if err.is_context_window_exceeded() {
            return false;
        }
        return matches!(
            err,
            archon_llm::provider::LlmError::Http(_)
                | archon_llm::provider::LlmError::RateLimited { .. }
                | archon_llm::provider::LlmError::Overloaded
                | archon_llm::provider::LlmError::Server { .. }
        );
    }

    let message = error.to_string().to_ascii_lowercase();
    [
        "http error",
        "error decoding response body",
        "connection reset",
        "connection closed",
        "connection refused",
        "timed out",
        "timeout",
        "temporarily unavailable",
        "server overloaded",
        "rate limited",
        "429",
        "500",
        "502",
        "503",
        "504",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn pipeline_attempt_retry_delay(attempt: usize) -> Duration {
    let exponent = (attempt as u32).saturating_sub(1);
    let delay_ms = 250u64.saturating_mul(2u64.saturating_pow(exponent));
    Duration::from_millis(delay_ms.min(2_000))
}

fn fail_audit(audit: &mut Option<PipelineAuditRun>, error: &str) -> Result<()> {
    if let Some(audit) = audit.as_mut() {
        audit.fail(error)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -- format_leann_results ------------------------------------------------

    #[test]
    fn format_leann_results_empty_input_returns_empty_string() {
        let results: Vec<archon_leann::SearchResult> = vec![];
        assert_eq!(format_leann_results(&results), "");
    }

    #[test]
    fn format_leann_results_single_result_has_correct_markdown() {
        let results = vec![archon_leann::SearchResult {
            file_path: PathBuf::from("src/main.rs"),
            content: "fn main() {}".to_string(),
            language: "rust".to_string(),
            line_start: 1,
            line_end: 3,
            relevance_score: 0.95,
        }];

        let output = format_leann_results(&results);

        assert!(
            output.starts_with("## Code Context\n"),
            "should start with header"
        );
        assert!(output.contains("`src/main.rs`"), "should contain file path");
        assert!(output.contains("lines 1-3"), "should contain line range");
        assert!(output.contains("```rust"), "should contain language fence");
        assert!(
            output.contains("fn main() {}"),
            "should contain code content"
        );
    }

    #[test]
    fn format_leann_results_multiple_results() {
        let results = vec![
            archon_leann::SearchResult {
                file_path: PathBuf::from("a.rs"),
                content: "fn a() {}".to_string(),
                language: "rust".to_string(),
                line_start: 1,
                line_end: 1,
                relevance_score: 0.9,
            },
            archon_leann::SearchResult {
                file_path: PathBuf::from("b.py"),
                content: "def b(): pass".to_string(),
                language: "python".to_string(),
                line_start: 10,
                line_end: 12,
                relevance_score: 0.7,
            },
        ];

        let output = format_leann_results(&results);

        // Both files should appear
        assert!(output.contains("`a.rs`"));
        assert!(output.contains("`b.py`"));
        assert!(output.contains("```rust"));
        assert!(output.contains("```python"));
    }

    // -- extract_modified_files ----------------------------------------------

    #[test]
    fn extract_modified_files_empty_log_returns_empty() {
        let log: Vec<ToolUseEntry> = vec![];
        assert!(extract_modified_files(&log).is_empty());
    }

    #[test]
    fn extract_modified_files_extracts_write_and_edit() {
        let log = vec![
            ToolUseEntry {
                tool_name: "Write".to_string(),
                input: serde_json::json!({ "file_path": "/src/a.rs", "content": "..." }),
                output: serde_json::json!({}),
            },
            ToolUseEntry {
                tool_name: "Edit".to_string(),
                input: serde_json::json!({ "file_path": "/src/b.rs", "old_string": "x", "new_string": "y" }),
                output: serde_json::json!({}),
            },
        ];

        let paths = extract_modified_files(&log);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/src/a.rs"));
        assert_eq!(paths[1], PathBuf::from("/src/b.rs"));
    }

    #[test]
    fn extract_modified_files_deduplicates() {
        let log = vec![
            ToolUseEntry {
                tool_name: "Write".to_string(),
                input: serde_json::json!({ "file_path": "/src/a.rs", "content": "v1" }),
                output: serde_json::json!({}),
            },
            ToolUseEntry {
                tool_name: "Edit".to_string(),
                input: serde_json::json!({ "file_path": "/src/a.rs", "old_string": "x", "new_string": "y" }),
                output: serde_json::json!({}),
            },
        ];

        let paths = extract_modified_files(&log);
        assert_eq!(paths.len(), 1, "duplicate paths should be deduplicated");
        assert_eq!(paths[0], PathBuf::from("/src/a.rs"));
    }

    #[test]
    fn extract_modified_files_ignores_other_tools() {
        let log = vec![
            ToolUseEntry {
                tool_name: "Read".to_string(),
                input: serde_json::json!({ "file_path": "/src/a.rs" }),
                output: serde_json::json!({}),
            },
            ToolUseEntry {
                tool_name: "Bash".to_string(),
                input: serde_json::json!({ "command": "ls" }),
                output: serde_json::json!({}),
            },
        ];

        assert!(extract_modified_files(&log).is_empty());
    }

    #[test]
    fn extract_modified_files_skips_missing_file_path() {
        let log = vec![ToolUseEntry {
            tool_name: "Write".to_string(),
            input: serde_json::json!({ "content": "orphan content, no file_path" }),
            output: serde_json::json!({}),
        }];

        assert!(extract_modified_files(&log).is_empty());
    }

    // -- LeannIntegration (unit-level, no DB) --------------------------------

    // NOTE: Full integration tests for LeannIntegration require a CozoDB
    // instance and are covered in integration test files. Here we verify
    // the helper functions that do not need a live DB.

    #[test]
    fn leann_integration_search_context_formats_correctly() {
        // This test exercises format_leann_results indirectly through the
        // struct method — we cannot construct a LeannIntegration without a
        // CodeIndex, but we can verify the formatting path separately.
        let results = vec![archon_leann::SearchResult {
            file_path: PathBuf::from("lib.rs"),
            content: "pub fn hello() {}".to_string(),
            language: "rust".to_string(),
            line_start: 5,
            line_end: 7,
            relevance_score: 0.85,
        }];

        let formatted = format_leann_results(&results);
        assert!(formatted.contains("## Code Context"));
        assert!(formatted.contains("lib.rs"));
        assert!(formatted.contains("lines 5-7"));
    }
}
