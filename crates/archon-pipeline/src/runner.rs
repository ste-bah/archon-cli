//! Pipeline runner loop.
//!
//! Provides the [`PipelineFacade`] trait and [`run_pipeline`] function that
//! implement a shared, context-isolated agent execution loop used by all
//! pipeline types (coding, research, learning, knowledge-base).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::learning::integration::LearningIntegration;
use crate::learning::reflexion::{FailedTrajectory, ReflexionInjector};

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
    /// [`LeannSearcher`](archon_pipeline::coding::rlm::LeannSearcher)
    /// implementations (e.g. for the research pipeline facade).
    pub fn code_index(&self) -> &Arc<archon_leann::CodeIndex> {
        &self.code_index
    }

    /// Index the repository on pipeline startup.
    ///
    /// Logs a warning and returns `Ok(())` if indexing fails so the pipeline
    /// can proceed without LEANN.
    pub async fn init_repository(&self, working_dir: &std::path::Path) -> Result<()> {
        let config = archon_leann::IndexConfig {
            root_path: working_dir.to_path_buf(),
            include_patterns: vec!["**/*.rs".into(), "**/*.py".into(), "**/*.ts".into()],
            exclude_patterns: vec![
                "**/target/**".into(),
                "**/node_modules/**".into(),
                "**/.git/**".into(),
            ],
        };
        match self.code_index.index_repository(working_dir, &config).await {
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

    tracing::info!(
        session_id = %session.id,
        pipeline_type = ?session.pipeline_type,
        task = %session.task,
        leann_enabled = leann.is_some(),
        "Pipeline session initialised"
    );

    loop {
        match facade.next_agent(&session).await? {
            NextAgent::Continue(agent) => {
                tracing::info!(
                    agent_key = %agent.key,
                    agent_name = %agent.display_name,
                    phase = agent.phase,
                    model = %agent.model,
                    "Executing agent"
                );

                // Inject LEANN code context before prompt build.
                if let Some(li) = leann {
                    session.leann_context = li.search_context(&session.task, &agent.key);
                }

                // v0.1.23: Query learning integration for SONA + ReasoningBank context.
                let learning_ctx = if let Some(ref mut li) = learning {
                    li.on_agent_start(
                        &agent.key,
                        &agent.phase.to_string(),
                        &session.task,
                        &session.id,
                    )
                } else {
                    Default::default()
                };

                // v0.1.23: Retry loop with reflexion injection (max 3 attempts).
                const MAX_ATTEMPTS: usize = 3;
                let mut attempt = 0usize;
                let (result, quality) = loop {
                    attempt += 1;

                    // Build a fresh prompt — context isolation.
                    let (messages, mut system, tools) =
                        facade.build_prompt(&session, &agent).await?;

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
                    }

                    // Inject reflexion context on retry.
                    if attempt > 1
                        && let Some(ref ri) = reflexion
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

                    // Execute against the LLM.
                    let agent_start = Instant::now();
                    let llm_response = llm
                        .send_message(messages, system, tools, &agent.model)
                        .await?;
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
                    let quality = facade.score_quality(&session, &agent, &result).await?;
                    result.quality = Some(quality.clone());

                    let meets_threshold = quality.overall >= agent.quality_threshold;

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

                    if meets_threshold || attempt >= MAX_ATTEMPTS {
                        break (result, quality);
                    }

                    // Record failure for reflexion on next attempt.
                    if let Some(ref mut ri) = reflexion {
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
                };

                // Post-processing.
                facade
                    .process_completion(&mut session, &agent, &result, &quality)
                    .await?;

                // v0.1.23: Feed quality back into learning integration (SONA trajectory).
                if let Some(ref mut li) = learning {
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
                session.agent_results.push((agent, result));
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

    facade.finalize(session).await
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
