//! Research pipeline facade implementing [`PipelineFacade`].
//!
//! Wires together [`ResearchPromptBuilder`], [`PhDQualityCalculator`], and
//! [`StyleInjector`] to drive the 47-agent research pipeline through the
//! shared runner loop.
//!
//! # Memory
//!
//! Per REQ-RESEARCH-008, agent outputs are persisted via `archon-memory`
//! (CozoDB + HNSW) with tags `["phd-pipeline", "<namespace>"]`. LEANN
//! semantic search provides fallback for missing keys.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

use anyhow::{Context, Result};
use async_trait::async_trait;

use archon_memory::{MemoryTrait, MemoryType, SearchFilter};

use crate::leann_searcher::LeannSearcher;
use crate::prompt_cap::{PromptBudget, PromptLayer, TruncationPriority, truncate_prompt_to_budget};
use crate::runner::{
    AgentInfo, AgentResult, NextAgent, PipelineFacade, PipelineResult, PipelineSession,
    PipelineType, QualityScore, ToolAccessLevel,
};

use crate::learning::integration::PhDLearningIntegration;

use super::agents::{RESEARCH_AGENTS, ResearchAgent, get_agent_by_key};
use super::final_assembly;
use super::prompt_builder::ResearchPromptBuilder;
use super::quality::PhDQualityCalculator;
use super::rlm::{ResearchRlm, research_output_namespaces};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Tag applied to all research pipeline memories.
const TAG_PHD_PIPELINE: &str = "phd-pipeline";

// ---------------------------------------------------------------------------
// ResearchFacade
// ---------------------------------------------------------------------------

/// Facade that drives the 47-agent PhD research pipeline.
pub struct ResearchFacade {
    quality_calculator: PhDQualityCalculator,
    prompt_builder: ResearchPromptBuilder,
    /// CozoDB + HNSW memory backend per REQ-RESEARCH-008.
    memory: Arc<dyn MemoryTrait>,
    /// LEANN semantic search fallback for missing memory keys.
    leann_searcher: Option<Arc<dyn LeannSearcher>>,
    /// Project path for memory provenance.
    project_path: String,
    /// Optional style override provided via `--style`.
    style_prompt: Option<String>,
    /// Optional PhD learning integration for recording quality feedback.
    learning: Option<Mutex<PhDLearningIntegration>>,
    /// Run-level memory for god-research style rolling context.
    rlm_store: Mutex<ResearchRlm>,
    /// Optional sender for per-agent progress events (TUI streaming).
    /// Uses internal mutability so the sender can be attached after
    /// construction (it's not known at bootstrap time).
    tui_sender: Mutex<Option<UnboundedSender<String>>>,
    /// Anthropic model alias map. Defaults to compile-time defaults; callers
    /// that have an active `ArchonConfig` should pass `config.models.anthropic`
    /// via `with_models(..)` so operator overrides apply.
    models: archon_core::config::AnthropicModelsConfig,
    context: archon_core::config::ContextConfig,
}

impl ResearchFacade {
    /// Create a new facade backed by the given memory backend.
    pub fn new(
        memory: Arc<dyn MemoryTrait>,
        leann_searcher: Option<Arc<dyn LeannSearcher>>,
        project_path: String,
        style_prompt: Option<String>,
    ) -> Self {
        Self {
            quality_calculator: PhDQualityCalculator::new(),
            prompt_builder: ResearchPromptBuilder::new(),
            memory,
            leann_searcher,
            project_path,
            style_prompt,
            learning: None,
            rlm_store: Mutex::new(ResearchRlm::new()),
            tui_sender: Mutex::new(None),
            models: archon_core::config::AnthropicModelsConfig::default(),
            context: archon_core::config::ContextConfig::default(),
        }
    }

    /// Create a new facade with PhD learning integration enabled.
    pub fn with_learning(
        memory: Arc<dyn MemoryTrait>,
        leann_searcher: Option<Arc<dyn LeannSearcher>>,
        project_path: String,
        style_prompt: Option<String>,
        learning: PhDLearningIntegration,
    ) -> Self {
        Self {
            quality_calculator: PhDQualityCalculator::new(),
            prompt_builder: ResearchPromptBuilder::new(),
            memory,
            leann_searcher,
            project_path,
            style_prompt,
            learning: Some(Mutex::new(learning)),
            rlm_store: Mutex::new(ResearchRlm::new()),
            tui_sender: Mutex::new(None),
            models: archon_core::config::AnthropicModelsConfig::default(),
            context: archon_core::config::ContextConfig::default(),
        }
    }

    /// Attach a TUI sender at construction time (builder pattern).
    pub fn with_tui_sender(mut self, tx: UnboundedSender<String>) -> Self {
        self.tui_sender = Mutex::new(Some(tx));
        self
    }

    /// Attach an operator-configured Anthropic model alias map (builder pattern).
    pub fn with_models(mut self, models: archon_core::config::AnthropicModelsConfig) -> Self {
        self.models = models;
        self
    }

    pub fn with_context(mut self, context: archon_core::config::ContextConfig) -> Self {
        self.context = context;
        self
    }

    /// Set the TUI sender after construction (called from dispatch handler).
    pub fn set_tui_sender(&self, tx: UnboundedSender<String>) {
        *self.tui_sender.lock().expect("tui_sender lock") = Some(tx);
    }

    /// Extract the top-level namespace from a memory key for tagging.
    ///
    /// `"research/foundation/framing"` → `"research"`.
    fn key_namespace(key: &str) -> &str {
        key.split('/').next().unwrap_or("research")
    }

    /// Persist a value under the given memory key with `phd-pipeline` tags.
    fn store_memory(&self, key: &str, value: String) {
        let namespace = Self::key_namespace(key);
        let tags: Vec<String> = vec![TAG_PHD_PIPELINE.to_string(), namespace.to_string()];
        let _ = self.memory.store_memory(
            &value,
            key,
            MemoryType::Fact,
            0.5,
            &tags,
            "pipeline",
            &self.project_path,
        );
    }

    /// Recall content for a memory key, with LEANN fallback.
    fn recall_memory(&self, key: &str) -> String {
        // Search by phd-pipeline tag, filter by title match.
        let filter = SearchFilter {
            tags: vec![TAG_PHD_PIPELINE.to_string()],
            ..Default::default()
        };
        if let Ok(memories) = self.memory.search_memories(&filter) {
            for m in &memories {
                if m.title == key {
                    return m.content.clone();
                }
            }
        }

        // LEANN fallback.
        if let Some(ref leann) = self.leann_searcher {
            return leann.search(key);
        }

        String::new()
    }

    /// Recall memory and audited prior outputs for an agent.
    fn recall_prior_context(&self, session: &PipelineSession, agent: &ResearchAgent) -> String {
        let mut parts = Vec::new();

        if let Ok(rlm) = self.rlm_store.lock() {
            let context = rlm.build_context(session, agent);
            if !context.is_empty() {
                parts.push(context);
            }
        }

        for namespace in research_output_namespaces(agent) {
            let content = self.recall_memory(&namespace);
            if !content.is_empty() {
                parts.push(format!("### Persistent Memory: `{namespace}`\n\n{content}"));
            }
        }

        parts.join("\n\n---\n\n")
    }

    /// Convert a [`ResearchAgent`] to an [`AgentInfo`].
    ///
    /// Emits the tier alias verbatim; resolution to a concrete model id
    /// happens at the provider boundary via `LlmProvider::resolve_alias(..)`.
    fn to_agent_info(
        agent: &ResearchAgent,
        _models: &archon_core::config::AnthropicModelsConfig,
    ) -> AgentInfo {
        let tool_access = if agent.phase >= 6 {
            ToolAccessLevel::Full
        } else {
            ToolAccessLevel::ReadOnly
        };

        AgentInfo {
            key: agent.key.to_string(),
            display_name: agent.display_name.to_string(),
            model: "sonnet".to_string(),
            phase: agent.phase as u32,
            critical: super::quality::PhDQualityCalculator::create_quality_context(
                agent.key,
                agent.phase,
            )
            .is_critical_agent,
            parallelizable: false,
            quality_threshold: 0.50,
            tool_access_level: tool_access,
        }
    }
}

#[async_trait]
impl PipelineFacade for ResearchFacade {
    async fn init_session(&self, task: &str) -> Result<PipelineSession> {
        let session_id = uuid::Uuid::new_v4().to_string();
        Ok(PipelineSession {
            id: session_id,
            pipeline_type: PipelineType::Research,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: Vec::new(),
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, session: &PipelineSession) -> Result<NextAgent> {
        let idx = session.agent_results.len();
        let final_idx = final_assembly::STATIC_AGENTS_BEFORE_FINAL;
        if idx < final_idx {
            let agent = &RESEARCH_AGENTS[idx];
            return Ok(NextAgent::Continue(Self::to_agent_info(
                agent,
                &self.models,
            )));
        }
        if let Some(agent) = final_assembly::next_final_stage_agent(session)? {
            return Ok(NextAgent::Continue(agent));
        }
        if idx >= RESEARCH_AGENTS.len() || final_idx < idx {
            return Ok(NextAgent::Done);
        }
        let agent = &RESEARCH_AGENTS[idx];
        Ok(NextAgent::Continue(Self::to_agent_info(
            agent,
            &self.models,
        )))
    }

    async fn build_prompt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        self.build_prompt_for_attempt(session, agent, 1).await
    }

    async fn build_prompt_for_attempt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
        attempt: u8,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        let style = self.style_prompt.as_deref();

        let prompt_text = if final_assembly::is_dynamic_final_key(&agent.key) {
            final_assembly::build_final_stage_prompt(session, &agent.key, &session.task, style)?
        } else {
            let research_agent = get_agent_by_key(&agent.key)
                .with_context(|| format!("Unknown research agent key: {}", agent.key))?;
            let prior_context = self.recall_prior_context(session, research_agent);
            self.prompt_builder.build(
                research_agent,
                session.agent_results.len(),
                RESEARCH_AGENTS.len(),
                &session.task,
                &prior_context,
                style,
            )
        };
        let context_window = archon_llm::context_window::resolve_context_window(
            &agent.model,
            self.context
                .context_window_override
                .or_else(|| self.context.max_tokens.map(u64::from)),
            None,
        )
        .context_window as usize;
        let budget = PromptBudget::from_context_config(context_window, &self.context, attempt);
        let truncated = truncate_prompt_to_budget(
            vec![PromptLayer {
                name: "research_prompt".to_string(),
                content: prompt_text,
                priority: TruncationPriority::Required,
                required: true,
            }],
            budget.max_prompt_tokens,
        )
        .context("research prompt truncation failed")?;
        let prompt_text = truncated
            .layers
            .into_iter()
            .map(|layer| layer.content)
            .collect::<Vec<_>>()
            .join("\n\n");

        let messages = vec![serde_json::json!({
            "role": "user",
            "content": prompt_text,
        })];

        let system = vec![serde_json::json!({
            "type": "text",
            "text": format!(
                "You are the {} agent in the PhD Research Pipeline. \
                 Follow the instructions carefully and produce high-quality academic output.",
                agent.display_name
            ),
        })];

        let tools: Vec<serde_json::Value> = Vec::new();

        Ok((messages, system, tools))
    }

    async fn score_quality(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
    ) -> Result<QualityScore> {
        if final_assembly::is_dynamic_final_key(&agent.key) {
            return Ok(final_assembly::score_final_stage_output(
                session,
                &agent.key,
                &result.output,
            ));
        }
        let ctx = PhDQualityCalculator::create_quality_context(&agent.key, agent.phase as u8);
        let assessment = self.quality_calculator.assess_quality(&result.output, &ctx);

        let mut dimensions = HashMap::new();
        dimensions.insert(
            "content_depth".to_string(),
            assessment.breakdown.content_depth,
        );
        dimensions.insert(
            "structural_quality".to_string(),
            assessment.breakdown.structural_quality,
        );
        dimensions.insert(
            "research_rigor".to_string(),
            assessment.breakdown.research_rigor,
        );
        dimensions.insert(
            "completeness".to_string(),
            assessment.breakdown.completeness,
        );
        dimensions.insert(
            "format_quality".to_string(),
            assessment.breakdown.format_quality,
        );
        let mut overall = assessment.score;
        if super::citation_gate::hard_failure(&agent.key, &result.output).is_some() {
            dimensions.insert("citation_gate".to_string(), 0.0);
            overall = 0.0;
        }

        Ok(QualityScore {
            overall,
            dimensions,
        })
    }

    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
        quality: &QualityScore,
    ) -> Result<()> {
        if final_assembly::is_dynamic_final_key(&agent.key) {
            self.store_memory(
                &format!("research/final-stage/{}", agent.key),
                result.output.clone(),
            );
        }

        // Store output at agent's run-level and persistent memory keys.
        if let Some(research_agent) = get_agent_by_key(&agent.key)
            && let Ok(mut rlm) = self.rlm_store.lock()
        {
            rlm.write_agent_output(research_agent, _session.agent_results.len(), &result.output);
        }

        // Store output at all declared memory keys -- persisted via
        // CozoDB + HNSW with tags per REQ-RESEARCH-008.
        if let Some(research_agent) = get_agent_by_key(&agent.key) {
            for namespace in research_output_namespaces(research_agent) {
                self.store_memory(&namespace, result.output.clone());
            }
        }

        // Feed quality to PhD learning subsystem
        if let Some(ref learning_mutex) = self.learning
            && let Ok(mut learning) = learning_mutex.lock()
        {
            learning.record_citation_quality(&agent.key, quality.overall);
        }

        // Emit per-agent progress to TUI if sender is attached.
        if let Some(ref tx) = *self.tui_sender.lock().expect("tui_sender lock") {
            let _ = tx.send(format!(
                "[pipeline phase {}] {} complete (quality: {:.2})\n",
                agent.phase, agent.display_name, quality.overall,
            ));
        }

        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> Result<PipelineResult> {
        final_assembly::assemble_result(session)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
#[path = "facade_tests.rs"]
mod tests;
